use crate::exports::plugin::injector::guest::PluginError;
use crate::message_utils::trim_messages;
use crate::plugin::injector::env::env_var;
use crate::plugin::injector::error::FsError;
use crate::plugin::injector::host::{
    call_plugin, log, mcp_meta_tools, mcp_tools, new_client, streaming_enabled, MetaTools,
};
use crate::plugin::injector::logger::Level;
use crate::plugin::injector::open_a_i_like::{
    ChatConfig, ChatSession, Client, ContentType, FunctionUsage, Message, MessageContent,
    ToolCallUsage,
};
use crate::types::*;
use crate::SseEvent;
use serde_json::Value;
use shared::TryFromEnvVar;
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub fn parse_chat_request(input: &str) -> Result<ChatRequest, PluginError> {
    serde_json::from_str(input).map_err(|e| PluginError::Json(format!("Invalid input json: {}", e)))
}

pub fn setup_config(
    req: &ChatRequest,
) -> Result<(shared::types::LLMConfig, String, bool), PluginError> {
    // Get LLM config
    let llm_config = match &req.llm_config {
        Some(config) => config.clone(),
        None => shared::types::LLMConfig::try_from_env_var("LLM_CONFIG")
            .map_err(|e| PluginError::EnvVar(format!("Failed to load LLM_CONFIG: {}", e)))?,
    };

    // Get system context
    let system_context = match &req.system_context {
        Some(context) => {
            log(Level::Info, &format!("provided context: {}", context));
            context.clone()
        }
        None => match env_var("SYSTEM_CONTEXT") {
            Ok(ctx) => ctx,
            Err(e) => {
                log(
                    Level::Warn,
                    &format!("Failed to load SYSTEM_CONTEXT: {}", e),
                );
                String::default()
            }
        },
    };

    // Ensure directories exist
    let had_memories = ensure_directories_exist()?;

    Ok((llm_config, system_context, had_memories))
}

fn ensure_directories_exist() -> Result<bool, PluginError> {
    // Create memories.md if it doesn't exist
    let memories_path = "memories.md";
    let mut had_memories = true;
    if !Path::new(memories_path).exists() {
        log(
            Level::Info,
            &format!("Creating memories file at {}", memories_path),
        );
        had_memories = false;
        fs::write(memories_path, "{}").map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to create memories file: {}",
                e
            )))
        })?;
    } // Check if memories file is empty or just contains {}
    let memories_content = fs::read_to_string(memories_path).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to read memories file: {}",
            e
        )))
    })?;

    if memories_content.trim().is_empty() {
        had_memories = false;
        log(Level::Info, "Memories file exists but is empty");
    }

    // Ensure threads directory exists
    let threads_dir = "threads".to_string();
    if !Path::new(&threads_dir).exists() {
        fs::create_dir_all(&threads_dir).map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to create threads directory: {}",
                e
            )))
        })?;
    }

    Ok(had_memories)
}

pub fn initialize_chat_session(
    req: &ChatRequest,
    llm_config: &shared::types::LLMConfig,
    system_context: &str,
    had_memories: bool,
) -> Result<(ChatSession, String, bool, Option<String>), PluginError> {
    let tools = setup_tools(llm_config)?;

    let chat_config = ChatConfig {
        model: llm_config.model_name.clone(),
        temperature: llm_config.temperature,
        max_tokens: llm_config.max_tokens,
        top_p: llm_config.top_p,
        top_k: llm_config.top_k,
        tools: Some(tools),
        tool_choice: None,
        messages: vec![],
        streaming: Some(streaming_enabled()),
        response_schema: llm_config.response_schema.clone(),
    };

    // Create client
    let client = new_client(&llm_config.base_url, &llm_config.api_key)?;

    // Determine thread ID - use existing or create a new one
    let thread_id = req
        .thread_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let thread_file_path = format!("threads/thread-{}.json", thread_id);

    let mut is_new_thread = false;
    let mut title = None;
    let chat_session;

    // Try to load existing thread if thread_id was provided
    if let Some(ref _thread_id) = req.thread_id {
        if Path::new(&thread_file_path).exists() {
            // Load existing thread
            (chat_session, title) =
                load_existing_thread(&thread_file_path, &chat_config, client, system_context)?;
        } else {
            log(
                Level::Warn,
                &format!(
                    "Requested thread with ID {} does not exist, creating new thread",
                    thread_id
                ),
            );
            chat_session = ChatSession::new(&chat_config, client, true);
            is_new_thread = true;
        }
    } else {
        chat_session = ChatSession::new(&chat_config, client, true);
        is_new_thread = true;
    }

    if !had_memories {
        let new_user_context = r#"""
        You are chatting with a new user - before following the user's direction, steer the conversation such that the user answers all of the following questions.

        Be really aggressive with saving information gained to memory. Work really hard to figure out how the user wants to work with you. Tell them about being able to build long lived documents,
        interact with integrated software, and provide seamless integration with their existing tools. Always end with a follow-up question until we have all the information we need.

        We want to remember all of this:

        1. **Introduction**: Begin by welcoming the user and introducing yourself as their assistant.

        2. **Gather Personal Information**:
        - Ask for their name and preferred title/role.
        - Inquire about their responsibilities within the company.

        3. **Work Preferences**:
        - Ask what types of tasks they are most interested in receiving assistance with (e.g., project management, coding, documentation).
        - Determine their preferred tools or software for task management.

        4. **Communication Style**:
        - Inquire how they prefer to receive updates (e.g., concise summaries vs. detailed explanations).

        5. **Project Participation**:
        - Ask about any ongoing projects they are involved in and how they would like to contribute.

        6. **Collaborators**:
        - Ask if there are team members or collaborators they work closely with, and how they prefer to refer to them.

        7. **File Management Preferences**:
        - Inquire about their preferred file formats for tasks (e.g., Markdown for readable content, JSON for data storage).
        - Ask how they would like tasks and information to be organized.

        8. **Feedback Loop**:
        - Encourage them to add any additional information or preferences that are important for their work.

        9. **Memories Storage**:
        - Inform the user that their responses will be stored in a memories file for future reference, enhancing the assistant’s ability to provide tailored support.

        ---

        Ensure that the conversation is engaging, empathetic, and supportive while gathering the necessary information for effective onboarding. Adapt your responses based on their input, and remember to be flexible in addressing their needs.
        """#;
        chat_session.add_message(&Message {
            role: "system".to_string(),
            content: ContentType::Single(MessageContent::Content(new_user_context.to_string())),
            tool_calls: None,
            tool_call_id: None,
        })?;
    }

    // For new threads, always add the system message if it's not empty
    if is_new_thread && !system_context.is_empty() {
        log(
            Level::Info,
            &format!("Adding system context: {}", system_context),
        );

        let memory_md = fs::read_to_string("memories.md").unwrap_or_else(|_| "".to_string());

        chat_session.add_message(&Message {
            role: "system".to_string(),
            content: ContentType::Single(MessageContent::Content(
                system_context.to_string()
                    + "--- Memories ---\n"
                    + &memory_md
                    + "\n--- Current Time (UTC) ---\n"
                    + &chrono::Utc::now().to_rfc3339(),
            )),
            tool_calls: None,
            tool_call_id: None,
        })?;
    }

    Ok((chat_session, thread_id, is_new_thread, title))
}

fn setup_tools(llm_config: &shared::types::LLMConfig) -> Result<Vec<String>, PluginError> {
    let meta_tools = mcp_meta_tools()?;

    // Create meta-tool getter
    let mut tools = create_meta_tool_schema(&meta_tools)?;

    // Add default tools if specified
    add_default_tools(&mut tools)?;

    // If the config has custom tools, use those instead
    if let Some(custom_tools) = &llm_config.tools {
        return Ok(custom_tools.clone());
    }

    Ok(tools)
}

fn create_meta_tool_schema(meta_tools: &MetaTools) -> Result<Vec<String>, PluginError> {
    let schemas = meta_tools.tools.iter().map(|tool| {
        let mut t = tool.clone();
        t.name = t.name.replace(".", "_");
        serde_json::json!({
            "type": "function",
            "function": {
                "name": format!("get_tools_for_{}", t.name),
                "description": format!("Gets the tools related to {} which: {}", t.name, t.description),
            }
        })
    }).collect::<Vec<Value>>();

    // let schema = serde_json::json!({
    //     "type": "function",
    //     "function": {
    //         "name": "conc-meta-tool-getter",
    //         "description": "Adds the tool/function to the available tools to call by the llm",
    //         "parameters": {
    //             "type": "object",
    //             "properties": {
    //                 "get": {
    //                     "type": "array",
    //                     "description": "The tools to get",
    //                     "items": {
    //                         "type": "string",
    //                         "enum": meta_tools.tools.iter().map(|tool| {
    //                             let mut t = tool.clone();
    //                             t.name = t.name.replace(".", "_");
    //                             serde_json::to_string(&t).unwrap()
    //                         }).collect::<Vec<String>>()
    //                     }
    //                 }
    //             }
    //         }
    //     }
    // });

    log(
        Level::Info,
        &format!(
            "Tools: {}",
            serde_json::to_string_pretty(&schemas).unwrap_or_default()
        ),
    );
    Ok(schemas.into_iter().map(|tool| tool.to_string()).collect())
}

fn add_default_tools(tools: &mut Vec<String>) -> Result<(), PluginError> {
    let default_tool_ids: Vec<String> =
        serde_json::from_str(&env_var("DEFAULT_TOOL_IDS").unwrap_or_default()).unwrap_or_default();

    if default_tool_ids.is_empty() {
        return Ok(());
    }

    log(
        Level::Info,
        &format!("Adding default tools: {:?}", default_tool_ids),
    );

    let full_tools_str = mcp_tools()?;
    let full_tools: Vec<Value> = serde_json::from_str(&full_tools_str).map_err(|e| {
        PluginError::Json(format!(
            "Invalid tools json: {}, input: {}",
            e, full_tools_str
        ))
    })?;

    for tool_id in default_tool_ids {
        if let Some(tool) = full_tools.iter().find(|t| {
            if let Some(function) = t.get("function") {
                if let Some(name) = function.get("name") {
                    return name.as_str().unwrap_or("") == tool_id;
                }
            }
            false
        }) {
            let mut tool = tool.clone();
            let name = tool
                .as_object_mut()
                .unwrap()
                .get_mut("function")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .get_mut("name")
                .unwrap();
            *name = Value::String(name.as_str().unwrap().replace(".", "_"));
            let tool_json = serde_json::to_string(&tool).unwrap_or_default();
            tools.push(tool_json);
        }
    }

    Ok(())
}

fn load_existing_thread(
    thread_file_path: &str,
    chat_config: &ChatConfig,
    client: Client,
    system_context: &str,
) -> Result<(ChatSession, Option<String>), PluginError> {
    log(
        Level::Info,
        &format!("Loading existing thread from {}", thread_file_path),
    );

    let thread_data = fs::read_to_string(thread_file_path).map_err(|e| {
        PluginError::Fs(FsError::Other(format!("Failed to read thread file: {}", e)))
    })?;

    let thread_storage: ThreadStorage = serde_json::from_str(&thread_data)
        .map_err(|e| PluginError::Json(format!("Invalid thread data: {}", e)))?;

    let title = thread_storage.title;
    let chat_session = if title.is_some() {
        ChatSession::new(chat_config, client, false)
    } else {
        ChatSession::new(chat_config, client, true)
    };

    // Check for existing system message
    let (existing_system_msg, existing_system_content) =
        find_existing_system_message(&thread_storage.messages);

    let should_add_system_msg = existing_system_msg.is_none();
    let should_update_system_msg = existing_system_msg.is_some()
        && existing_system_content.is_some()
        && existing_system_content.unwrap() != system_context
        && !system_context.is_empty();

    // Add system context if needed (either new or updated) and it's not empty
    if (should_add_system_msg || should_update_system_msg) && !system_context.is_empty() {
        let system_message = Message {
            role: existing_system_msg.unwrap_or_else(|| "system".to_string()),
            content: ContentType::Single(MessageContent::Content(system_context.to_string())),
            tool_calls: None,
            tool_call_id: None,
        };
        chat_session.add_message(&system_message)?;
    }

    // Add stored messages, replacing system message if needed
    for message in thread_storage.messages {
        // Skip the old system message if we need to update it
        if should_update_system_msg && (message.role == "system" || message.role == "developer") {
            continue;
        }
        chat_session.add_message(&message)?;
    }

    Ok((chat_session, title))
}

fn find_existing_system_message(messages: &[Message]) -> (Option<String>, Option<String>) {
    let mut existing_system_msg = None;
    let mut existing_system_content = None;

    for msg in messages {
        if msg.role == "system" || msg.role == "developer" {
            existing_system_msg = Some(msg.role.clone());
            if let ContentType::Single(MessageContent::Content(content)) = &msg.content {
                existing_system_content = Some(content.clone());
            }
            break;
        }
    }

    (existing_system_msg, existing_system_content)
}

pub fn process_user_message(
    chat_session: &mut ChatSession,
    user_message_text: &str,
) -> Result<String, PluginError> {
    // Add the user message
    let user_message = Message {
        role: "user".to_string(),
        content: ContentType::Single(MessageContent::Content(user_message_text.to_string())),
        tool_calls: None,
        tool_call_id: None,
    };
    chat_session.add_message(&user_message)?;

    // Trim messages to stay under token limit
    let mut messages = chat_session.messages();
    let original_length = messages.len();
    trim_messages(&mut messages).map_err(|e| PluginError::ChatCompletion(e.to_string()))?;
    if messages.len() < original_length {
        log(
            Level::Info,
            &format!(
                "Trimmed messages from {} to {}",
                original_length,
                messages.len()
            ),
        );
    }
    chat_session.set_messages(&messages)?;

    // Send message and handle any tool calls
    let response_text = process_chat_with_tool_calls(chat_session)?;

    Ok(response_text)
}

fn process_chat_with_tool_calls(chat_session: &mut ChatSession) -> Result<String, PluginError> {
    let pre_tool_call_idx = chat_session.messages().len() as u64;
    let mut response = chat_session.send()?;

    let mut choice = response
        .choices
        .first()
        .ok_or_else(|| PluginError::ChatCompletion("No response from OpenAI".to_string()))?;

    log(Level::Info, &format!("Initial response: {:?}", choice));

    let full_tools_str = mcp_tools()?;
    let full_tools: Vec<Value> = serde_json::from_str(&full_tools_str)
        .map_err(|e| PluginError::Json(format!("Invalid tools json: {}", e)))?;

    // Keep performing tool calls until we get a normal message back
    while let Some(ref tool_calls) = choice.message.tool_calls {
        log(
            Level::Info,
            &format!("Found {} tool calls", tool_calls.len()),
        );

        for tool_call in tool_calls {
            process_tool_call(tool_call, chat_session, pre_tool_call_idx, &full_tools)?;
        }

        // Trim messages to stay under token limit
        let mut messages = chat_session.messages();
        // Check for empty tool call arguments and set to "{}"
        for message in messages.iter_mut() {
            if let Some(tool_calls) = &mut message.tool_calls {
                for tool_call in tool_calls.iter_mut() {
                    if tool_call.function.arguments.is_empty() {
                        tool_call.function.arguments = "{}".to_string();
                        log(
                            Level::Info,
                            &format!(
                                "Fixed empty tool call arguments for: {}",
                                tool_call.function.name
                            ),
                        );
                    }
                }
            }
        }

        let original_length = messages.len();
        log(
            Level::Info,
            &format!("Messages length before trimming: {}", original_length),
        );
        trim_messages(&mut messages).map_err(|e| PluginError::ChatCompletion(e.to_string()))?;
        chat_session.set_messages(&messages)?;

        // Get next response
        response = chat_session.send()?;

        choice = response
            .choices
            .first()
            .ok_or_else(|| PluginError::ChatCompletion("No response from OpenAI".to_string()))?;

        log(
            Level::Info,
            &format!("New response after tool calls: {:?}", choice),
        );
    }

    // Extract final response text
    let response_text: String = choice.message.content.as_ref().cloned().unwrap_or_default();

    Ok(response_text)
}

fn process_tool_call(
    tool_call: &ToolCallUsage,
    chat_session: &mut ChatSession,
    pre_tool_call_idx: u64,
    full_tools: &Vec<Value>,
) -> Result<(), PluginError> {
    log(Level::Info, &format!("Tool call received: {:?}", tool_call));

    if tool_call.function.name.starts_with("get_tools_for_") {
        process_meta_tool_getter(tool_call, chat_session, full_tools)?;
    } else {
        // For all other tools, call the plugin
        process_regular_tool_call(
            tool_call,
            chat_session,
            pre_tool_call_idx,
            full_tools,
            false,
        )?;
    }

    Ok(())
}

fn split_any_of_tool(schema: Value) -> Vec<Value> {
    // Check if this is a schema that needs to be split
    if let Some(function) = schema.get("function") {
        if let Some(parameters) = function.get("parameters") {
            if let Some(properties) = parameters.get("properties") {
                // Find properties that have anyOf
                let mut any_of_prop = None;
                let mut any_of_variations = 0;

                for (prop_name, prop_value) in
                    properties.as_object().unwrap_or(&serde_json::Map::new())
                {
                    if let Some(any_of) = prop_value.get("anyOf") {
                        if any_of.as_array().map_or(0, |a| a.len()) > any_of_variations {
                            any_of_prop = Some(prop_name.clone());
                            any_of_variations = any_of.as_array().unwrap().len();
                        }
                    }
                }

                // If we found an anyOf property, split the tool
                if let Some(prop_name) = any_of_prop {
                    if let Some(any_of) = properties.get(&prop_name).and_then(|p| p.get("anyOf")) {
                        if let Some(any_of_array) = any_of.as_array() {
                            let base_name = function
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown");

                            return any_of_array.iter().filter_map(|variant| {
                                // // Extract the variant's action type and description
                                let variant_desc = variant.get("description").and_then(|d| d.as_str()).unwrap_or("");

                                // if its enum:
                                if let Some(enum_variants) = variant.get("enum") {
                                    let enum_name = enum_variants[0].as_str()?;
                                    return Some(serde_json::json!({
                                        "type": "function",
                                        "function": {
                                            "name": format!("{}_sub_tool_{enum_name}", base_name),
                                            "description": variant_desc
                                        }
                                    }));
                                }

                                // if its object:
                                // Look for properties that have the first key as an enum
                                let variant_props = variant.get("properties")?.as_object()?;
                                let mut action_key = None;

                                // Iterate through properties to find first enum
                                for (_, value) in variant_props {
                                    if let Some(enum_variants) = value.get("enum") {
                                        if enum_variants.as_array().unwrap().len() == 1 {
                                            if let Some(likely_key) = enum_variants[0].as_str() {
                                                action_key = Some(likely_key.to_string());
                                                break;
                                            }
                                        }
                                    }
                                }

                                // Fallback to first key if no enum found
                                let action_key = action_key.or_else(|| variant_props.keys().next().cloned())?;

                                // Create a new function for this variant
                                let new_function = serde_json::json!({
                                    "type": "function",
                                    "function": {
                                        "name": format!("{}_sub_tool_{}", base_name, action_key.replace(".", "_")),
                                        "description": variant_desc,
                                        "parameters": {
                                            "type": "object",
                                            "properties": {
                                                prop_name.clone(): variant
                                            },
                                            "required": [prop_name],
                                            "additionalProperties": false,
                                            "strict": true
                                        }
                                    }
                                });

                                // Log the new function for debugging
                                log(
                                    Level::Info,
                                    &format!(
                                        "Created new function from anyOf split: {}",
                                        serde_json::to_string_pretty(&new_function).unwrap_or_default()
                                    ),
                                );
                                Some(new_function)
                            }).collect();
                        } else {
                            log(Level::Info, "anyOf not array?");
                        }
                    } else {
                        log(Level::Info, "No anyOf items");
                    }
                } else {
                    log(Level::Info, "No anyOf");
                }
            } else {
                log(Level::Info, "No properties");
            }
        } else {
            log(Level::Info, "No parameters");
        }
    } else {
        log(Level::Info, "No function");
    }

    // If no anyOf to split, return the original schema as a single-element vector
    vec![schema]
}

fn process_meta_tool_getter(
    tool_call: &ToolCallUsage,
    chat_session: &mut ChatSession,
    full_tools: &Vec<Value>,
) -> Result<(), PluginError> {
    log(Level::Info, "Processing conc-meta-tool-getter request");

    let tool_name = if tool_call.function.name.starts_with("get_tools_for_") {
        let name = tool_call
            .function
            .name
            .strip_prefix("get_tools_for_")
            .unwrap_or("");
        log(Level::Info, &format!("Extracted tool category: {}", name));
        name
    } else {
        log(Level::Warn, "Function name doesn't match expected pattern");
        return Err(PluginError::ChatCompletion(
            "Function name doesn't match expected pattern".to_string(),
        ));
    };

    // Add requested tools to the session
    for tool in full_tools {
        let name = tool
            .as_object()
            .unwrap()
            .get("function")
            .unwrap()
            .as_object()
            .unwrap()
            .get("name")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
            .replace(".", "_");

        if tool_name == name {
            let mut tool_clone = tool.clone();
            *tool_clone
                .as_object_mut()
                .unwrap()
                .get_mut("function")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .get_mut("name")
                .unwrap() = Value::String(name.replace(".", "_"));

            let pretty_tool_schema = serde_json::to_string_pretty(&tool_clone).unwrap();
            log(
                Level::Info,
                &format!("Adding requested tool: {}", pretty_tool_schema),
            );

            let res = split_any_of_tool(tool_clone);
            log(
                Level::Info,
                &format!(
                    "Post-split requested tool: {}",
                    serde_json::to_string_pretty(&res).unwrap()
                ),
            );
            res.into_iter().try_for_each(|schema| {
                chat_session.add_tool(&serde_json::to_string(&schema).unwrap())?;
                Ok(())
            })?;
        }
    }

    // Add empty response for the tool
    chat_session.add_message(&Message {
        role: "tool".to_string(),
        content: ContentType::Single(MessageContent::Content(
            "Tools added successfully".to_string(),
        )),
        tool_call_id: Some(tool_call.id.clone()),
        tool_calls: None,
    })?;

    Ok(())
}

fn process_regular_tool_call(
    tool_call: &ToolCallUsage,
    chat_session: &mut ChatSession,
    pre_tool_call_idx: u64,
    full_tools: &Vec<Value>,
    already_tried_schemaed: bool,
) -> Result<(), PluginError> {
    let real_func_name = tool_call
        .function
        .name
        .split("_sub_tool_")
        .next()
        .map(|s| s.to_string())
        .unwrap_or(tool_call.function.name.clone());
    // Call the actual plugin
    let res = call_plugin(&real_func_name, &tool_call.function.arguments);

    let res_str = match res {
        Ok(res) => res,
        Err(err) => {
            log(Level::Warn, &format!("Tool call err: {err}"));
            if !already_tried_schemaed && err.to_string().contains("PluginError::Json") {
                return fix_tool_call(tool_call, chat_session, pre_tool_call_idx, full_tools);
            }

            if err.to_string().contains("is not accessible") {
                format!(
                    "Error: \"{}\" - the function does not exist. try calling the associated `get_tools_for_<function_name>` that closest matches what you tried to call.",
                    err
                )
            } else if err.to_string().contains("missing field") {
                // For now, return the original error message with a hint
                format!(
                    "Error: \"{}\" - hint: Try calling the `get_tools_for_{}` first to get the whole schema",
                    err, tool_call.function.name
                )
            } else {
                format!("Error: \"{}\"", err)
            }
        }
    };

    log(
        Level::Info,
        &format!(
            "Tool call response: {}",
            serde_json::from_str::<Value>(&res_str)
                .as_ref()
                .map(|v| serde_json::to_string_pretty(v).unwrap())
                .unwrap_or_else(|_| res_str.clone())
        ),
    );

    if streaming_enabled() {
        let event = SseEvent::new();
        event.set_data(
            &serde_json::to_string(&serde_json::json!({
                "set_tool_call_success": {
                    "id": tool_call.id,
                    "success": !res_str.starts_with("Error:")
                }
            }))
            .unwrap(),
        )?;
        event.send()?;
    }

    // Add tool response to chat
    chat_session.add_message(&Message {
        role: "tool".to_string(),
        content: ContentType::Single(MessageContent::Content(res_str)),
        tool_call_id: Some(tool_call.id.clone()),
        tool_calls: None,
    })?;

    Ok(())
}

fn fix_tool_call(
    tool_call: &ToolCallUsage,
    chat_session: &mut ChatSession,
    pre_tool_call_idx: u64,
    full_tools: &Vec<Value>,
) -> Result<(), PluginError> {
    log(
        Level::Info,
        &format!(
            "Tool call failed with schema error, attempting to get correct schema for tool '{:?}'",
            tool_call
        ),
    );

    let real_func_name = tool_call
        .function
        .name
        .split("_sub_tool_")
        .next()
        .map(|s| s.to_string())
        .unwrap_or(tool_call.function.name.clone());

    // Try to find the schema for this specific tool
    let mut schema = full_tools
        .iter()
        .find_map(|tool| {
            if let Some(function) = tool.get("function") {
                if let Some(name) = function.get("name") {
                    if name.as_str().unwrap_or("") == real_func_name {
                        return function.get("parameters").cloned();
                    }
                }
            }
            None
        })
        .unwrap_or_else(|| serde_json::json!({}));

    log(
        Level::Info,
        &format!(
            "Using schema: {}",
            serde_json::to_string_pretty(&schema).unwrap_or_default()
        ),
    );

    schema.as_object_mut().unwrap().remove("examples");

    let schema = serde_json::json!({
        "name": "root",
        "strict": true,
        "schema": schema
    })
    .to_string();
    let schema_chat_session = chat_session.fork_at(pre_tool_call_idx)?;
    schema_chat_session.set_response_schema(Some(&schema))?;
    schema_chat_session.remove_all_tools()?;
    schema_chat_session.disable_streaming();
    schema_chat_session.add_message(&Message {
        role: "assistant".to_string(),
        content: ContentType::Single(MessageContent::Content(format!(
            "Let me try to construct valid JSON to call the function: {} with the arguments: {}",
            tool_call.function.name, tool_call.function.arguments
        ))),
        tool_call_id: None,
        tool_calls: None,
    })?;
    let response = schema_chat_session.send()?;

    // Log the response for debugging purposes
    log(
        Level::Info,
        &format!("Schema chat session response: {:?}", response),
    );

    let choice = response
        .choices
        .first()
        .ok_or_else(|| PluginError::ChatCompletion("No schema response generated".to_string()))?;

    let fixed_tool_usage = ToolCallUsage {
        function: FunctionUsage {
            name: tool_call.function.name.clone(),
            arguments: choice.message.content.as_ref().unwrap().clone(),
        },
        id: tool_call.id.clone(),
        tool_type: tool_call.tool_type.clone(),
    };

    // Update the tool call in the chat session's messages
    let mut messages = chat_session.messages();
    'm: for message in messages.iter_mut().rev() {
        if let Some(ref mut tool_calls) = &mut message.tool_calls {
            for tool_call in tool_calls.iter_mut() {
                if tool_call.id == fixed_tool_usage.id {
                    *tool_call = fixed_tool_usage.clone();
                    break 'm;
                }
            }
        }
    }

    // Replace the message in the chat session
    chat_session.set_messages(&messages)?;
    log(
        Level::Info,
        &format!(
            "Updated tool call with fixed schema arguments: {}",
            fixed_tool_usage.function.arguments
        ),
    );

    log(Level::Info, &format!("Schema chat choice: {:?}", choice));
    process_regular_tool_call(
        &fixed_tool_usage,
        chat_session,
        pre_tool_call_idx,
        full_tools,
        true,
    )
}
