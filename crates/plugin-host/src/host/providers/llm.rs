use crate::{
    injector::{
        error::PluginError,
        open_a_i_like::{
            ChatCompletion, ChatConfig, ContentType, EmbeddingInput, EncodingFormat, Message,
            MessageContent, MessageResponse, ToolSelection,
        },
    },
    server::SseStreamTx,
};
use async_openai::{
    config::{Config, OpenAIConfig as InnerOpenAIConfig},
    types::{
        CreateEmbeddingRequest, CreateEmbeddingResponse, EmbeddingInput as InnerEmbeddingInput,
        EncodingFormat as InnerEncodingFormat,
    },
    Client as InnerClient,
};
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest_eventsource::{Event, RequestBuilderExt};
use std::fmt::Display;
use uuid::Uuid;

use crate::{replace, select};

/// Wrapper around OpenAI's configuration.
#[derive(Debug, Clone, Default)]
pub struct OpenAIConfig(pub InnerOpenAIConfig);

/// Client for making requests to OpenAI-compatible APIs.
#[derive(Clone)]
pub struct Client(pub InnerClient<InnerOpenAIConfig>);

impl Client {
    /// Creates a new client with the given configuration.
    pub fn with_config(config: OpenAIConfig) -> Self {
        Client(InnerClient::with_config(config.0))
    }
}

/// Extends Result with convenience methods for plugin error conversion
trait PluginErrorExt<T, E: Display> {
    /// Converts any error to a PluginError::ChatCompletion with the given context
    fn chat_err<C: Into<String>>(self, context: C) -> Result<T, PluginError>;

    /// Converts any error to a PluginError::Json with the given context
    fn json_err<C: Into<String>>(self, context: C) -> Result<T, PluginError>;
}

impl<T, E: Display> PluginErrorExt<T, E> for Result<T, E> {
    fn chat_err<C: Into<String>>(self, context: C) -> Result<T, PluginError> {
        self.map_err(|e| PluginError::ChatCompletion(format!("{}: {}", context.into(), e)))
    }

    fn json_err<C: Into<String>>(self, context: C) -> Result<T, PluginError> {
        self.map_err(|e| PluginError::Json(format!("{}: {}", context.into(), e)))
    }
}

/// Helper method to convert HTTP status codes to appropriate error messages
fn status_to_error_message(status: StatusCode, body: &str) -> String {
    match status {
        StatusCode::UNAUTHORIZED => format!("Authentication failed: {}", body),
        StatusCode::FORBIDDEN => format!("Access denied: {}", body),
        StatusCode::NOT_FOUND => format!("Resource not found: {}", body),
        StatusCode::TOO_MANY_REQUESTS => format!("Rate limit exceeded: {}", body),
        StatusCode::BAD_REQUEST => format!("Bad request: {}", body),
        status if status.is_server_error() => {
            format!("Server error ({}): {}", status.as_u16(), body)
        }
        _ => format!("Request failed with status {}: {}", status.as_u16(), body),
    }
}

/// Creates a chat completion using the provided client and configuration.
pub async fn chat_create(
    client: &mut Client,
    config: &ChatConfig,
    sse_sender: &Option<SseStreamTx>,
) -> Result<ChatCompletion, PluginError> {
    // Build the request JSON
    let json = construct_chat_request_json(client, config)?;

    tracing::info!(
        "Sending request to LLM: {}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );

    let http_client = reqwest::Client::builder()
        .build()
        .chat_err("Failed to build HTTP client")?;

    let endpoint = format!("{}/chat/completions", client.0.config().api_base());

    // Handle streaming if enabled
    if should_stream(config, sse_sender) {
        tracing::info!("Using stream chat completion");
        match stream_chat_completion(
            http_client.clone(),
            endpoint.clone(),
            client.0.config(),
            json.clone(),
            sse_sender,
        )
        .await
        {
            Ok(res) => return Ok(res),
            Err(e) => {
                tracing::warn!("Failed to stream chat completion: {}", e);
                // Fall through to non-streaming path
            }
        }
    }

    // Non-streaming path
    let mut json = json;
    json["stream"] = Some(false).into();
    let response =
        send_non_streaming_request(http_client, endpoint, client.0.config(), json).await?;
    process_chat_completion_response(response)
}

/// Determines if streaming should be used
fn should_stream(config: &ChatConfig, sse_sender: &Option<SseStreamTx>) -> bool {
    matches!(config.streaming, Some(true)) && sse_sender.is_some()
}

/// Builder for constructing chat request JSON
struct ChatRequestBuilder<'a> {
    client: &'a Client,
    config: &'a ChatConfig,
    json: serde_json::Value,
}

impl<'a> ChatRequestBuilder<'a> {
    fn new(client: &'a Client, config: &'a ChatConfig) -> Self {
        let json = serde_json::json!({
            "messages": config.messages.iter().map(|msg| {
                let mut json = serde_json::json!({
                    "role": msg.role,
                    "content": match &msg.content {
                        ContentType::Array(parts) => {
                            let content_values: Vec<_> = parts.iter().map(|part| match part {
                                MessageContent::Parts(parts) => serde_json::to_value(parts).unwrap(),
                                MessageContent::Content(content) => serde_json::to_value(content).unwrap(),
                            }).collect();
                            serde_json::to_value(content_values).unwrap()
                        },
                        ContentType::Single(content) => match content {
                            MessageContent::Parts(parts) => serde_json::to_value(parts).unwrap(),
                            MessageContent::Content(content) => serde_json::to_value(content).unwrap(),
                        },
                    }
                });

                // Add tool_call_id if present
                if let Some(tool_call_id) = &msg.tool_call_id {
                    json.as_object_mut().unwrap().insert(
                        "tool_call_id".to_string(),
                        serde_json::Value::String(tool_call_id.clone())
                    );
                }

                // Process tool_calls if present
                if let Some(ref tool_calls) = msg.tool_calls {
                    let tool_calls_array = tool_calls.iter().map(|tool_call_usage| {
                        let mut as_json = serde_json::to_value(tool_call_usage).unwrap();
                        let obj = as_json.as_object_mut().unwrap();
                        // Convert from our internal "tool_type" field to API's "type" field
                        let ty = obj.remove("tool_type").unwrap();
                        obj.insert("type".to_string(), ty);
                        as_json
                    }).collect();

                    json.as_object_mut().unwrap().insert(
                        "tool_calls".to_string(),
                        serde_json::Value::Array(tool_calls_array)
                    );
                }
                json
            }).collect::<Vec<_>>(),
            "model": config.model,
            "top_p": config.top_p,
            "temperature": config.temperature,
            "max_tokens": config.max_tokens,
            "stream": config.streaming,
            "tools": config.tools.as_ref().map(|tools| Some(tools.iter().map(|tool| {
                serde_json::from_str::<serde_json::Value>(tool).unwrap_or_default()
            }).collect::<Vec<_>>())),
        });

        Self {
            client,
            config,
            json,
        }
    }

    fn apply_tool_choice(mut self) -> Self {
        if let Some(ref tool_choice) = self.config.tool_choice {
            match tool_choice {
                ToolSelection::Auto => {
                    self.json["tool_choice"] = serde_json::Value::String("auto".to_string())
                }
                ToolSelection::Required => {
                    self.json["tool_choice"] = serde_json::Value::String("required".to_string())
                }
                ToolSelection::Forced(ref tool_name) => {
                    self.json["tool_choice"] =
                        serde_json::json!({"type": "function", "function": {"name": tool_name}})
                }
            }
        }
        self
    }

    fn remove_null_values(mut self) -> Self {
        if let Some(obj) = self.json.as_object_mut() {
            // Collect keys to remove to avoid borrowing issues
            let keys_to_remove: Vec<String> = obj
                .iter()
                .filter(|(_, v)| v.is_null())
                .map(|(k, _)| k.clone())
                .collect();

            // Remove all null values
            for key in keys_to_remove {
                obj.remove(&key);
            }
        }
        self
    }

    fn apply_response_schema(mut self) -> Result<Self, PluginError> {
        if let Some(ref schema_str) = self.config.response_schema {
            let mut schema = serde_json::from_str::<serde_json::Value>(schema_str)
                .json_err("Response format was not valid JSON")?;

            // Different API providers use different schema formats
            let api_base = self.client.0.config().api_base();
            if api_base.contains("openai.com/v1") || api_base.contains(":1234") {
                // OpenAI or LM Studio format
                recurse_openai_format(schema.get_mut("schema").unwrap(), true, false)
                    .json_err("Invalid response format")?;
                self.json.as_object_mut().unwrap().insert(
                    "response_format".to_string(),
                    serde_json::json!({
                        "type": "json_schema",
                        "json_schema": schema,
                    }),
                );
            } else if api_base.contains(":11434") {
                // Ollama format
                self.json
                    .as_object_mut()
                    .unwrap()
                    .insert("format".to_string(), schema);
            } else {
                // Default format (Fireworks, Together, etc.)
                self.json.as_object_mut().unwrap().insert(
                    "response_format".to_string(),
                    serde_json::json!({
                        "type": "json_object",
                        "schema": schema
                    }),
                );
            }
        }
        Ok(self)
    }

    fn build(self) -> Result<serde_json::Value, PluginError> {
        Ok(self.json)
    }
}

fn construct_chat_request_json(
    client: &Client,
    config: &ChatConfig,
) -> Result<serde_json::Value, PluginError> {
    ChatRequestBuilder::new(client, config)
        .apply_tool_choice()
        .remove_null_values()
        .apply_response_schema()?
        .build()
}

/// https://platform.openai.com/docs/guides/structured-outputs#supported-schemas
fn recurse_openai_format(
    val: &mut serde_json::Value,
    is_root: bool,
    make_ty_nullable: bool,
) -> Result<(), String> {
    // remove all supported keywords
    if val.as_object_mut().is_some() {
        let t = val.as_object_mut().unwrap();
        t.remove("format");
        t.remove("minimum");
        t.remove("maximum");
        t.remove("minLength");
        t.remove("maxLength");
        t.remove("multipleOf");
        t.remove("patternProperties");
        t.remove("unevaluatedProperties");
        t.remove("propertyNames");
        t.remove("minProperties");
        t.remove("maxProperties");
        t.remove("unevaluatedItems");
        t.remove("minItems");
        t.remove("maxItems");
        t.remove("minContains");
        t.remove("maxContains");
        t.remove("contains");
        t.remove("uniqueItems");
    }

    // Check if this is an object and add additionalProperties: false if needed
    if val.get("type").is_some_and(|t| t == "object") {
        if val.get("additionalProperties").is_none() {
            val["additionalProperties"] = serde_json::json!(false);
        }

        // Enforce all properties to be required
        if let Some(properties) = val.get("properties") {
            // {
            //   "type": "object"
            //   "properties": {
            //     ..
            //   }
            // }
            let property_names: Vec<String> = properties
                .as_object()
                .ok_or_else(|| "Properties must be an object".to_string())?
                .keys()
                .cloned()
                .collect();

            let start_required = val.get("required").cloned();
            // Create required array if it doesn't exist
            if val.get("required").is_none() {
                val["required"] = serde_json::json!(property_names);
            } else if let Some(required) = val.get_mut("required") {
                // Make sure all properties are in the required array
                let current_required = required
                    .as_array_mut()
                    .ok_or_else(|| "Required must be an array".to_string())?;
                for name in property_names {
                    if !current_required.iter().any(|r| r.as_str() == Some(&name)) {
                        current_required.push(serde_json::json!(name));
                    }
                }
            }

            // Recurse into properties
            if let Some(props) = val.get_mut("properties") {
                let props_obj = props
                    .as_object_mut()
                    .ok_or_else(|| "Properties must be an object".to_string())?;

                // Get the current required properties if any exist
                for (prop_name, prop_val) in props_obj.iter_mut() {
                    // If we have a required array and this property wasn't in it,
                    // mark it as nullable (make_ty_nullable = true)
                    let make_nullable = start_required
                        .as_ref()
                        .map(|set| {
                            !set.as_array()
                                .unwrap()
                                .iter()
                                .any(|v| v.as_str() == Some(prop_name))
                        })
                        .unwrap_or(false);

                    recurse_openai_format(prop_val, false, make_nullable)?;
                }
            }
        }
    }

    if val.get("type").is_some_and(|t| t == "array") {
        // Process array items
        if let Some(items) = val.get_mut("items") {
            recurse_openai_format(items, false, false)?;
        }
    }

    // Make nullable if needed
    if make_ty_nullable && val.get("type").is_some() {
        let current_type = val["type"].clone();
        if current_type.is_string() {
            // Convert string type to array of types including null
            val["type"] = serde_json::json!([current_type, "null"]);
        } else if current_type.is_array() {
            // Add null to array of types if not already present
            let mut types = current_type.as_array().unwrap().clone();
            if !types.iter().any(|t| t == "null") {
                types.push(serde_json::json!("null"));
                val["type"] = serde_json::json!(types);
            }
        }
    }

    // Process $defs if present
    if let Some(defs) = val.get_mut("$defs") {
        let defs_obj = defs
            .as_object_mut()
            .ok_or_else(|| "$defs must be an object".to_string())?;
        for (_, def_val) in defs_obj.iter_mut() {
            recurse_openai_format(def_val, false, false)?;
        }
    }

    if let Some(any_of) = val.get_mut("anyOf") {
        // Root objects must not be anyOf
        if is_root {
            return Err("Root level object must not use anyOf".to_string());
        }

        let any_of_arr = any_of
            .as_array_mut()
            .ok_or_else(|| "anyOf must be an array".to_string())?;
        for def_val in any_of_arr.iter_mut() {
            recurse_openai_format(def_val, false, false)?;
        }
    }

    // Check $ref for recursive schemas
    if val.get("$ref").is_some() {
        // $ref is allowed, nothing to modify
    }

    Ok(())
}

/// Handles streaming chat completion responses
async fn stream_chat_completion(
    http_client: reqwest::Client,
    endpoint: String,
    config: &InnerOpenAIConfig,
    json: serde_json::Value,
    sse_sender: &Option<SseStreamTx>,
) -> Result<ChatCompletion, PluginError> {
    let tx = sse_sender.as_ref().ok_or_else(|| {
        PluginError::ChatCompletion("Streaming enabled but no sender provided".to_string())
    })?;

    tracing::info!("Starting streaming response");

    // Create an empty response to build up
    let mut final_response = create_empty_response(&json);

    let mut event_source = http_client
        .post(endpoint)
        .query(&config.query())
        .headers(config.headers())
        .json(&json)
        .eventsource()
        .chat_err("Failed to create event source")?;

    // Helper function for sending errors to the SSE stream and returning an error
    let send_error = |tx: &SseStreamTx, error: &str| -> Result<ChatCompletion, PluginError> {
        // Try to send the error to the client
        if let Err(send_err) = tx.send(Err(PluginError::Generic(error.to_string()))) {
            return Err(PluginError::ChatCompletion(format!(
                "Receiver was dropped: {}",
                send_err
            )));
        }
        // Return the error
        Err(PluginError::ChatCompletion(format!(
            "Error from LLM: {}",
            error
        )))
    };

    // Process streaming events
    while let Some(ev) = event_source.next().await {
        match ev {
            Err(e) => {
                tracing::warn!("Error receiving event: {}", e);
                return send_error(tx, &e.to_string());
            }
            Ok(Event::Message(message)) => {
                if message.data == "[DONE]" {
                    tracing::info!("LLM response completed");
                    let tool_calls_present = final_response
                        .choices
                        .first()
                        .map(|choice| choice.finish_reason == "tool_calls")
                        .unwrap_or_default();

                    if !tool_calls_present {
                        tracing::info!("Closing connection: {:#?}", final_response);
                        if let Err(e) =
                            tx.send(Ok(axum::response::sse::Event::default().data("[DONE]")))
                        {
                            return Err(PluginError::ChatCompletion(format!(
                                "Receiver was dropped: {}",
                                e
                            )));
                        }
                    }
                    break;
                }

                // Parse and handle the streaming response chunk
                // Parse the streaming response chunk
                match serde_json::from_str::<async_openai::types::CreateChatCompletionStreamResponse>(
                    &message.data,
                ) {
                    Ok(response) => {
                        // Update our final response with the new data
                        update_final_response(&mut final_response, &response);

                        // Forward the event to the client
                        let event_data = serde_json::to_string(&response).unwrap_or_default();
                        if let Err(e) =
                            tx.send(Ok(axum::response::sse::Event::default().data(event_data)))
                        {
                            return Err(PluginError::ChatCompletion(format!(
                                "Receiver was dropped: {}",
                                e
                            )));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse response chunk: {}, data: {}",
                            e,
                            message.data
                        );
                        // Continue processing - don't fail the whole stream for one bad chunk
                    }
                }
            }
            Ok(Event::Open) => continue,
        }
    }

    event_source.close();
    // Validate the final response
    if final_response.choices.is_empty() {
        tracing::warn!("Streaming response contained no choices");
        return Err(PluginError::ChatCompletion(
            "LLM response contained no choices".to_string(),
        ));
    }

    tracing::debug!(
        "Done streaming chat completion: {}",
        serde_json::to_string_pretty(&final_response).unwrap_or_default()
    );
    Ok(final_response)
}

/// Creates an empty response structure to build up during streaming
fn create_empty_response(request: &serde_json::Value) -> ChatCompletion {
    // Extract the model name from the request
    let model = request["model"].as_str().unwrap_or("unknown").to_string();

    ChatCompletion {
        id: Uuid::new_v4().to_string(),
        choices: Vec::new(),
        created: chrono::Utc::now().timestamp() as u64,
        model,
        object: "chat.completion".to_string(),
        service_tier: None,
        system_fingerprint: None,
        usage: crate::injector::open_a_i_like::Usage::default(),
    }
}

/// Updates the final response with streaming chunks
fn update_final_response(
    final_response: &mut ChatCompletion,
    chunk: &async_openai::types::CreateChatCompletionStreamResponse,
) {
    // Update fields from the chunk
    final_response.id = chunk.id.clone();
    final_response.created = chunk.created as u64;
    final_response.model = chunk.model.clone();
    final_response.system_fingerprint = chunk.system_fingerprint.clone();

    // Update usage if present
    if let Some(usage) = &chunk.usage {
        final_response.usage = usage.clone().into();
    }

    // Process each choice in the chunk
    for choice in &chunk.choices {
        // Add new choices or update existing ones
        if let Some(existing) = final_response
            .choices
            .iter_mut()
            .find(|c| c.index == choice.index as u64)
        {
            // Update existing choice
            merge_choice(existing, choice);
        } else {
            // Add new choice
            final_response.choices.push(choice_to_our_format(choice));
        }
    }
}

/// Merge a streaming choice into an existing one
fn merge_choice(
    existing: &mut crate::injector::open_a_i_like::Choice,
    chunk: &async_openai::types::ChatChoiceStream,
) {
    // Update finish reason if provided
    update_finish_reason(existing, chunk);

    // Merge text content
    merge_message_content(existing, chunk);

    // Handle tool calls
    merge_tool_calls(existing, chunk);
}

/// Update the finish reason of a choice if provided in the chunk
fn update_finish_reason(
    existing: &mut crate::injector::open_a_i_like::Choice,
    chunk: &async_openai::types::ChatChoiceStream,
) {
    if let Some(reason) = &chunk.finish_reason {
        existing.finish_reason = serde_json::to_value(reason)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
    }
}

/// Merge content and refusal from the delta into the existing message
fn merge_message_content(
    existing: &mut crate::injector::open_a_i_like::Choice,
    chunk: &async_openai::types::ChatChoiceStream,
) {
    // Merge content if present in the delta
    if let Some(content) = &chunk.delta.content {
        if let Some(existing_content) = existing.message.content.as_mut() {
            *existing_content = format!("{}{}", existing_content, content);
        } else {
            existing.message.content = Some(content.clone());
        }
    }

    // Merge refusal if present in the delta
    if let Some(refusal) = &chunk.delta.refusal {
        if let Some(existing_refusal) = existing.message.refusal.as_mut() {
            *existing_refusal = format!("{}{}", existing_refusal, refusal);
        } else {
            existing.message.refusal = Some(refusal.clone());
        }
    }

    // Update role if present in the delta
    if let Some(role) = &chunk.delta.role {
        existing.message.role = serde_json::to_value(role)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
    }
}

/// Merge tool calls from the delta into the existing message
fn merge_tool_calls(
    existing: &mut crate::injector::open_a_i_like::Choice,
    chunk: &async_openai::types::ChatChoiceStream,
) {
    // Handle tool calls
    if let Some(tool_calls) = &chunk.delta.tool_calls {
        if tool_calls.is_empty() {
            return;
        }

        // Create tool_calls vector if it doesn't exist
        if existing.message.tool_calls.is_none() {
            existing.message.tool_calls = Some(Vec::new());
        }

        let existing_tool_calls = existing.message.tool_calls.as_mut().unwrap();

        for new_tool_call in tool_calls {
            // Try to find an existing tool call with the same index
            let index = new_tool_call.index as usize;

            // Expand the existing_tool_calls vector if needed
            ensure_tool_call_exists(existing_tool_calls, index);

            let existing_tool_call = &mut existing_tool_calls[index];
            update_tool_call(existing_tool_call, new_tool_call);
        }
    }
}

/// Ensure the tool calls vector has an entry at the specified index
fn ensure_tool_call_exists(
    existing_tool_calls: &mut Vec<crate::injector::open_a_i_like::ToolCallUsage>,
    index: usize,
) {
    while existing_tool_calls.len() <= index {
        existing_tool_calls.push(crate::injector::open_a_i_like::ToolCallUsage {
            id: String::new(),
            tool_type: "function".to_string(),
            function: crate::injector::open_a_i_like::FunctionUsage {
                name: String::new(),
                arguments: String::new(),
            },
        });
    }
}

/// Update a tool call with new data from a streaming chunk
fn update_tool_call(
    existing_tool_call: &mut crate::injector::open_a_i_like::ToolCallUsage,
    new_tool_call: &async_openai::types::ChatCompletionMessageToolCallChunk,
) {
    // Update ID if present
    if let Some(id) = &new_tool_call.id {
        if !id.is_empty() {
            existing_tool_call.id = id.clone();
        }
    }

    // Update type if present
    if let Some(type_) = &new_tool_call.r#type {
        existing_tool_call.tool_type = serde_json::to_value(type_)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
    }

    // Update function if present
    if let Some(func) = &new_tool_call.function {
        // Update function name if present
        let name = func.name.as_ref().cloned().unwrap_or_default();
        if !name.is_empty() {
            existing_tool_call.function.name = name.clone();
        }

        // Append arguments if present
        merge_function_arguments(&mut existing_tool_call.function, func);
    }
}

/// Merge function arguments from a streaming chunk
fn merge_function_arguments(
    existing_function: &mut crate::injector::open_a_i_like::FunctionUsage,
    new_function: &async_openai::types::FunctionCallStream,
) {
    let args = new_function.arguments.as_ref().cloned().unwrap_or_default();
    if !args.is_empty() {
        // If this is a partial JSON fragment, we need to handle it carefully
        if existing_function.arguments.is_empty() {
            // First fragment, just set it
            existing_function.arguments = args;
        } else {
            // For streaming JSON, append the fragment
            existing_function.arguments.push_str(&args);
        }
    }
}

/// Convert a streaming choice to our format
fn choice_to_our_format(
    choice: &async_openai::types::ChatChoiceStream,
) -> crate::injector::open_a_i_like::Choice {
    // Create basic choice structure
    let mut our_choice = crate::injector::open_a_i_like::Choice {
        index: choice.index as u64,
        finish_reason: choice.finish_reason.as_ref().map_or(String::new(), |r| {
            serde_json::to_value(r)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        }),
        logprobs: None,
        message: crate::injector::open_a_i_like::MessageResponse {
            role: choice
                .delta
                .role
                .map(|r| {
                    serde_json::to_value(r)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .unwrap_or_else(|| "assistant".to_string()),
            content: choice.delta.content.clone(),
            refusal: choice.delta.refusal.clone(),
            annotations: None,
            tool_calls: None,
        },
    };

    // Add tool calls if present
    if let Some(tool_calls) = &choice.delta.tool_calls {
        our_choice.message.tool_calls = Some(
            tool_calls
                .iter()
                .map(|tc| crate::injector::open_a_i_like::ToolCallUsage {
                    id: tc.id.clone().unwrap_or_default(),
                    tool_type: serde_json::to_value(tc.r#type.clone().unwrap_or_default())
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string(),
                    function: crate::injector::open_a_i_like::FunctionUsage {
                        name: tc
                            .function
                            .as_ref()
                            .map(|f| f.name.clone().unwrap_or_default())
                            .unwrap_or_default(),
                        arguments: tc
                            .function
                            .as_ref()
                            .map(|f| f.arguments.clone().unwrap_or_default())
                            .unwrap_or_default(),
                    },
                })
                .collect(),
        );
    }

    our_choice
}

/// Sends a non-streaming chat completion request
async fn send_non_streaming_request(
    http_client: reqwest::Client,
    endpoint: String,
    config: &InnerOpenAIConfig,
    json: serde_json::Value,
) -> Result<serde_json::Value, PluginError> {
    let resp = http_client
        .post(endpoint)
        .query(&config.query())
        .headers(config.headers())
        .json(&json)
        .send()
        .await
        .chat_err("Failed to send chat request")?;

    match resp.status() {
        reqwest::StatusCode::OK => {
            // Get response text
            let resp_str = resp
                .text()
                .await
                .chat_err("Failed to extract response text")?;

            // Parse the JSON response
            let json_resp = serde_json::from_str(&resp_str)
                .chat_err(format!("Failed to parse response: {}", resp_str))?;

            tracing::info!(
                "LLM Response: {}",
                serde_json::to_string_pretty(&json_resp).unwrap_or_default()
            );
            Ok(json_resp)
        }
        _ => {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_default();
            Err(PluginError::ChatCompletion(status_to_error_message(
                status,
                &error_text,
            )))
        }
    }
}

/// Processes the chat completion response, fixing any field mappings needed
fn process_chat_completion_response(
    mut res: serde_json::Value,
) -> Result<ChatCompletion, PluginError> {
    // Process tool_calls if present in the response, converting between API "type" and our "tool_type"
    if let Ok(tool_calls) = select(&res, "$.choices[0].message.tool_calls[*]") {
        let tool_calls_vec = tool_calls.into_iter().cloned().collect::<Vec<_>>();
        for (i, mut tool_call) in tool_calls_vec.into_iter().enumerate() {
            if let Some(obj) = tool_call.as_object_mut() {
                if let Some(type_value) = obj.remove("type") {
                    obj.insert("tool_type".to_string(), type_value);

                    let path = format!("$.choices[0].message.tool_calls[{i}]");
                    res = replace(res, &path, tool_call)
                        .chat_err("Failed to replace tool_call field")?;
                }
            }
        }
    }

    // Convert to our ChatCompletion type
    serde_json::from_value(res.clone()).json_err(format!(
        "Failed to deserialize response - {}",
        serde_json::to_string_pretty(&res).unwrap_or_default()
    ))
}

/// Creates embeddings using the provided client and parameters.
pub async fn embeddings_create(
    client: &mut Client,
    model: String,
    input: EmbeddingInput,
    encoding_format: Option<EncodingFormat>,
    user: Option<String>,
    dimensions: Option<u32>,
) -> Result<Vec<Vec<f32>>, PluginError> {
    // Convert our EmbeddingInput to the API's type
    let input = match input {
        EmbeddingInput::Str(s) => InnerEmbeddingInput::String(s),
        EmbeddingInput::StrArray(s) => InnerEmbeddingInput::StringArray(s),
        EmbeddingInput::IntegerArray(s) => InnerEmbeddingInput::IntegerArray(s),
        EmbeddingInput::ArrayOfIntegerArray(s) => InnerEmbeddingInput::ArrayOfIntegerArray(s),
    };

    // Convert our EncodingFormat to the API's type
    let encoding_format = encoding_format.map(|EncodingFormat::Float| InnerEncodingFormat::Float);

    // Build the request
    let req = CreateEmbeddingRequest {
        model,
        input,
        encoding_format,
        user,
        dimensions,
    };

    // Send the request and process the response
    let res: CreateEmbeddingResponse =
        client.0.embeddings().create_byot(req).await.map_err(|e| {
            PluginError::EmbeddingError(format!(
                "Failed to create embeddings: {}",
                e.to_string().trim()
            ))
        })?;

    // Extract the embeddings from the response
    Ok(res
        .data
        .into_iter()
        .map(|embedding| embedding.embedding)
        .collect())
}

/// Converts a message response to a vector of Messages
pub fn message_response_to_messages(response: MessageResponse) -> Vec<Message> {
    let mut messages = vec![];

    if let Some(tool_calls) = &response.tool_calls {
        // Handle tool calls in the response
        // NOTE: there is no guarantee that the tool calls FOLLOW ITS OWN SPEC. meaning we have to
        // adjust the function name accordingly.

        let mut tool_calls = tool_calls.clone();
        tool_calls.iter_mut().for_each(|tool_call| {
            tool_call.function.name = tool_call.function.name.replace(".", "_");
        });
        messages.push(Message {
            role: "assistant".to_string(),
            content: ContentType::Single(MessageContent::Content("".to_string())),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        });
    } else if let Some(content) = response.content {
        // Handle normal content response
        messages.push(Message {
            role: response.role,
            content: ContentType::Single(MessageContent::Content(content)),
            tool_calls: None,
            tool_call_id: None,
        });
    } else if let Some(refusal) = response.refusal {
        // Handle refusal response
        messages.push(Message {
            role: response.role,
            content: ContentType::Single(MessageContent::Content(refusal)),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    messages
}

/// Chat session management
#[derive(Clone)]
pub struct ChatSession {
    pub session_id: String,
    pub generate_title: bool,
    pub session_title: Option<String>,
    pub config: ChatConfig,
    pub messages: Vec<Message>,
    pub client: Client,
}

impl ChatSession {
    /// Creates a new chat session
    pub fn new(config: ChatConfig, client: Client, generate_title: bool) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            generate_title,
            session_title: None,
            config,
            messages: Vec::new(),
            client,
        }
    }

    /// Returns the session ID
    pub fn session_id(&self) -> String {
        self.session_id.clone()
    }

    /// Returns the session title if available
    pub fn session_title(&self) -> Option<String> {
        self.session_title.clone()
    }

    /// Returns the current configuration
    pub fn config(&self) -> ChatConfig {
        self.config.clone()
    }

    /// Returns the current message history
    pub fn messages(&self) -> Vec<Message> {
        self.messages.clone()
    }

    /// Set streaming mode
    pub fn set_streaming(&mut self, enabled: bool) {
        self.config.streaming = Some(enabled);
    }

    /// Enables streaming mode
    pub fn enable_streaming(&mut self) {
        self.set_streaming(true);
    }

    /// Disables streaming mode
    pub fn disable_streaming(&mut self) {
        self.set_streaming(false);
    }

    /// Sets the message history
    pub fn set_messages(&mut self, messages: Vec<Message>) -> Result<bool, PluginError> {
        self.messages = messages;
        Ok(true)
    }

    /// Attempts to generate a title for the session based on the conversation
    async fn maybe_generate_title(&mut self) -> Result<(), PluginError> {
        // Skip if title generation is disabled or already has a title
        if !self.generate_title || self.session_title.is_some() {
            return Ok(());
        }

        // Extract text content from relevant messages
        let prompt = Self::extract_conversation_text(&self.config.messages);

        // Check if we have enough content to generate a title
        if prompt.is_empty() || !has_sufficient_tokens(&prompt, 200) {
            return Ok(());
        }

        // Create config for title generation
        let title_config = self.create_title_generation_config(&prompt)?;

        // Get a title from the LLM
        let resp = chat_create(&mut self.client, &title_config, &None).await?;
        self.session_title = resp
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone());

        Ok(())
    }

    /// Extracts text content from conversation messages
    /// Extracts text content from conversation messages
    fn extract_conversation_text(messages: &[Message]) -> String {
        messages
            .iter()
            .filter(|message| message.role == "user" || message.role == "assistant")
            .map(|message| match &message.content {
                ContentType::Array(ref parts) => parts
                    .iter()
                    .map(|part| match part {
                        MessageContent::Parts(ref parts) => parts.text.clone(),
                        MessageContent::Content(ref content) => content.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                ContentType::Single(ref content) => match content {
                    MessageContent::Parts(ref parts) => parts.text.clone(),
                    MessageContent::Content(ref content) => content.clone(),
                },
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn create_title_generation_config(&self, prompt: &str) -> Result<ChatConfig, PluginError> {
        // Create title generation message
        let title_prompt = format!(
            "Create a three word raw text title for the session, ensuring it is concise and informative but general enough to encompass where the conversation may go given this prompt -- DO NOT ADD ANY PREAMPLE OR ANYTHING. JUST THE 3 WORD RESPONSE: {prompt}",
        );

        let message = Message {
            role: "user".to_string(),
            content: ContentType::Single(MessageContent::Content(title_prompt)),
            tool_calls: None,
            tool_call_id: None,
        };

        // Set up config for title generation
        let mut config = self.config.clone();
        config.tools = None;
        config.streaming = Some(false);

        // Replace the last message with our title generation prompt
        if !config.messages.is_empty() {
            let last = config.messages.len() - 1;
            config.messages[last] = message;
        } else {
            config.messages.push(message);
        }

        Ok(config)
    }

    /// Prepares and executes a chat request
    async fn setup_and_chat(
        &mut self,
        sse_sender: &Option<SseStreamTx>,
    ) -> Result<ChatCompletion, PluginError> {
        // Update config with current messages
        self.config.messages = self.messages.clone();

        // Generate title if needed
        self.maybe_generate_title().await?;

        // Execute the chat request
        let resp = chat_create(&mut self.client, &self.config, sse_sender).await?;

        // Update messages with the response
        if let Some(choice) = resp.choices.first() {
            self.messages
                .extend(message_response_to_messages(choice.message.clone()));
        }

        // Clear messages from config to avoid duplication
        self.config.messages = vec![];

        Ok(resp)
    }

    /// Sends a chat message and gets a response
    pub async fn chat(
        &mut self,
        content: String,
        sse_sender: &Option<SseStreamTx>,
    ) -> Result<ChatCompletion, PluginError> {
        // Create and add the user message
        let user_message = Message {
            role: "user".to_string(),
            content: ContentType::Single(MessageContent::Content(content)),
            tool_calls: None,
            tool_call_id: None,
        };

        self.messages.push(user_message);

        // Process the chat
        self.setup_and_chat(sse_sender).await
    }

    /// Adds a tool to the chat configuration
    pub fn add_tool(&mut self, tool_schema: String) -> Result<bool, PluginError> {
        // Parse the tool schema to check its function name
        let new_tool = serde_json::from_str::<serde_json::Value>(&tool_schema)
            .map_err(|e| PluginError::Json(format!("Invalid tool schema: {}", e)))?;

        // Extract the function name from the new tool
        let new_name = new_tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .ok_or_else(|| PluginError::Json("Tool schema missing function.name".to_string()))?;

        // Check if a tool with this name already exists
        if let Some(tools) = &self.config.tools {
            for tool in tools {
                if let Ok(tool_json) = serde_json::from_str::<serde_json::Value>(tool) {
                    if let Some(function) = tool_json.get("function") {
                        if let Some(name) = function.get("name") {
                            if let Some(name_str) = name.as_str() {
                                if name_str == new_name {
                                    return Ok(false); // Tool already exists
                                }
                            }
                        }
                    }
                }
            }
        }

        // Add the tool if it doesn't exist yet
        if let Some(tools) = &mut self.config.tools {
            tools.push(tool_schema);
        } else {
            self.config.tools = Some(vec![tool_schema]);
        }

        Ok(true)
    }

    /// Removes a tool from the chat configuration
    pub fn remove_tool(&mut self, tool_name: String) -> Result<bool, PluginError> {
        if let Some(tools) = &mut self.config.tools {
            let initial_len = tools.len();
            tools.retain(|t| {
                if let Ok(tool_json) = serde_json::from_str::<serde_json::Value>(t) {
                    if let Some(function) = tool_json.get("function") {
                        if let Some(name) = function.get("name") {
                            if let Some(name_str) = name.as_str() {
                                return name_str != tool_name;
                            }
                        }
                    }
                }
                true
            });
            Ok(initial_len > tools.len())
        } else {
            Ok(false)
        }
    }

    /// Adds a message to the session history
    pub fn add_message(&mut self, message: Message) -> Result<bool, PluginError> {
        self.messages.push(message);
        Ok(true)
    }

    /// Sets the tool choice for the session
    pub fn set_tool_choice(
        &mut self,
        tool_choice: Option<ToolSelection>,
    ) -> Result<bool, PluginError> {
        self.config.tool_choice = tool_choice;
        Ok(true)
    }

    /// Sends a specific message and gets a response
    pub async fn send_message(
        &mut self,
        message: Message,
        sse_sender: &Option<SseStreamTx>,
    ) -> Result<ChatCompletion, PluginError> {
        self.messages.push(message);
        self.setup_and_chat(sse_sender).await
    }

    /// Sends the current message history without adding a new message
    pub async fn send(
        &mut self,
        sse_sender: &Option<SseStreamTx>,
    ) -> Result<ChatCompletion, PluginError> {
        self.setup_and_chat(sse_sender).await
    }

    pub fn fork_at(&mut self, idx: u64) -> Result<ChatSession, PluginError> {
        let mut forked = self.clone();
        forked.messages.truncate(idx as usize);
        Ok(forked)
    }

    pub fn set_response_schema(&mut self, schema: Option<String>) -> Result<bool, PluginError> {
        self.config.response_schema = schema;
        Ok(true)
    }

    pub fn remove_all_tools(&mut self) -> Result<bool, PluginError> {
        self.config.tools = None;
        Ok(true)
    }
}

/// Checks if a text has at least the specified number of tokens
fn has_sufficient_tokens(text: &str, min_tokens: usize) -> bool {
    match tiktoken_rs::o200k_base() {
        Ok(bpe) => {
            let tokens = bpe.encode_with_special_tokens(text);
            tokens.len() >= min_tokens
        }
        Err(_) => {
            // If encoding fails, default to false
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_sufficient_tokens() {
        let mut input = serde_json::json!(
        {
          "title": "SlackRequest",
          "type": "object",
          "properties": {
            "operation": {
              "anyOf": [
                {
                  "description": "Gets a list of all users in the workspace. This includes both invited users and deleted/deactivated users.",
                  "type": "string",
                  "enum": [
                    "users_list"
                  ]
                },
                {
                  "type": "object",
                  "properties": {
                    "users_info": {
                      "type": "object",
                      "properties": {
                        "user": {
                          "type": "string"
                        }
                      },
                      "required": [
                        "user"
                      ],
                      "additionalProperties": false,
                      "strict": true
                    }
                  },
                  "required": [
                    "users_info"
                  ],
                  "additionalProperties": false,
                  "strict": true
                },
                {
                  "description": "Post a message to a channel",
                  "type": "object",
                  "properties": {
                    "chat_post_message": {
                      "type": "object",
                      "properties": {
                        "channel": {
                          "description": "The channel ID to post to",
                          "type": "string"
                        },
                        "text": {
                          "description": "The message to post",
                          "type": "string"
                        },
                        "thread_ts": {
                          "description": "Provide another message's ts value to make this message a reply. Avoid using a reply's ts value; use its parent instead.",
                          "type": [
                            "string",
                            "null"
                          ]
                        },
                        "blocks": {
                          "description": "A JSON-based array of structured blocks, presented as a URL-encoded string.",
                          "type": [
                            "array",
                            "null"
                          ],
                          "items": {
                            "anyOf": [
                              {
                                "type": "string"
                              },
                              {
                                "type": "number"
                              },
                              {
                                "type": "boolean"
                              },
                              {
                                "type": "array",
                                "additionalProperties": false,
                                "items": {
                                  "type": [
                                    "string",
                                    "number",
                                    "boolean"
                                  ]
                                }
                              }
                            ]
                          }
                        }
                      },
                      "required": [
                        "channel",
                        "text",
                        "thread_ts",
                        "blocks"
                      ],
                      "additionalProperties": false,
                      "strict": true
                    }
                  },
                  "required": [
                    "chat_post_message"
                  ],
                  "additionalProperties": false,
                  "strict": true
                },
                {
                  "description": "Get conversation/channel history",
                  "type": "object",
                  "properties": {
                    "conversation_history": {
                      "type": "object",
                      "properties": {
                        "channel": {
                          "description": "The channel ID",
                          "type": "string"
                        },
                        "limit": {
                          "description": "The max number of messages to return",
                          "type": [
                            "integer",
                            "null"
                          ],
                          "format": "uint32",
                          "minimum": 0
                        },
                        "latest": {
                          "description": "The latest unix timestamp of messages to return",
                          "type": [
                            "string",
                            "null"
                          ]
                        },
                        "oldest": {
                          "description": "The oldest unix timestamp of messages to return",
                          "type": [
                            "string",
                            "null"
                          ]
                        },
                        "inclusive": {
                          "description": "Whether to include messages from the oldest and/or newest timestamp",
                          "type": [
                            "boolean",
                            "null"
                          ]
                        }
                      },
                      "required": [
                        "channel",
                        "limit",
                        "latest",
                        "oldest",
                        "inclusive"
                      ],
                      "additionalProperties": false,
                      "strict": true
                    }
                  },
                  "required": [
                    "conversation_history"
                  ],
                  "additionalProperties": false,
                  "strict": true
                },
                {
                  "description": "Get information about a channel",
                  "type": "object",
                  "properties": {
                    "channel_info": {
                      "type": "object",
                      "properties": {
                        "channel": {
                          "description": "The channel ID",
                          "type": "string"
                        }
                      },
                      "required": [
                        "channel"
                      ],
                      "additionalProperties": false,
                      "strict": true
                    }
                  },
                  "required": [
                    "channel_info"
                  ],
                  "additionalProperties": false,
                  "strict": true
                },
                {
                  "description": "List all channels the bot can see\n conversations.list",
                  "type": "object",
                  "properties": {
                    "get_channels": {
                      "type": "object",
                      "properties": {
                        "exclude_archived": {
                          "description": "Set to true to exclude archived channels from the list.",
                          "type": [
                            "boolean",
                            "null"
                          ]
                        },
                        "types": {
                          "description": "Mix and match channel types by providing a comma-separated list of any combination of public_channel, private_channel, mpim, im",
                          "type": [
                            "string",
                            "null"
                          ]
                        },
                        "limit": {
                          "description": "The maximum number of items to return. Default is 100.",
                          "type": [
                            "integer",
                            "null"
                          ],
                          "format": "uint32",
                          "minimum": 0
                        },
                        "cursor": {
                          "description": "Paginate through collections of data by setting the cursor parameter to a next_cursor attribute returned by a previous request's response_metadata",
                          "type": [
                            "string",
                            "null"
                          ]
                        }
                      },
                      "required": [
                        "exclude_archived",
                        "types",
                        "limit",
                        "cursor"
                      ],
                      "additionalProperties": false,
                      "strict": true
                    }
                  },
                  "required": [
                    "get_channels"
                  ],
                  "additionalProperties": false,
                  "strict": true
                },
                {
                  "description": "Join a channel. This is useful when the bot is not a member of the channel and needs to be able to read or send messages in it.",
                  "type": "object",
                  "properties": {
                    "join_channel": {
                      "type": "object",
                      "properties": {
                        "channel": {
                          "description": "The channel ID",
                          "type": "string"
                        }
                      },
                      "required": [
                        "channel"
                      ],
                      "additionalProperties": false,
                      "strict": true
                    }
                  },
                  "required": [
                    "join_channel"
                  ],
                  "additionalProperties": false,
                  "strict": true
                }
              ]
            }
          },
          "required": [
            "operation"
          ],
          "examples": [
            {
              "auth": null,
              "operation": "users_list"
            },
            {
              "auth": null,
              "operation": {
                "conversation_history": {
                  "channel": "C08JBUZGY4T",
                  "limit": 10,
                  "latest": null,
                  "oldest": null,
                  "inclusive": null
                }
              }
            }
          ],
          "additionalProperties": false,
          "strict": true
        });

        recurse_openai_format(&mut input, true, false);
        println!("{}", serde_json::to_string_pretty(&input).unwrap());
    }
}
