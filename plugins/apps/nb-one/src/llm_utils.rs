use std::fs;
use std::path::Path;

use crate::exports::plugin::injector::guest::PluginError;
use crate::graph_utils::default_tools;
use crate::graph_utils::GraphTrace;
use crate::plugin::injector::env::env_var;
use crate::plugin::injector::error::FsError;
use crate::plugin::injector::host::log;
use crate::plugin::injector::host::mcp_tools;
use crate::plugin::injector::host::streaming_enabled;
use crate::plugin::injector::host::Level;
use crate::NBChatRequest;
use daggy::NodeIndex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
// use serde_json::Value;
use crate::plugin::injector::host::new_client;
use crate::plugin::injector::open_a_i_like::{
    ChatConfig, ChatSession, Client, ContentType, Message, MessageContent,
};
use shared::types::LLMConfig;
use shared::TryFromEnvVar;

#[derive(Debug, Deserialize, Serialize)]
pub struct ThreadStorage {
    /// Optional title for the thread
    pub title: Option<String>,
    /// Collection of messages in the conversation thread
    pub messages: Vec<Message>,
    /// Current active node position in the graph
    pub current_node: NodeIndex,
    /// Record of graph traversal decisions made during the conversation
    pub graph_traces: Vec<GraphTrace>,
    /// Optional configuration for the chat system
    pub config: Option<ChatConfig>,
    /// Whether tools are loaded into the context
    pub tools_in_ctx: bool,
}

pub fn setup_config() -> Result<(shared::types::LLMConfig, String), PluginError> {
    let llm_config = LLMConfig::try_from_env_var("LLM_CONFIG")
        .map_err(|e| PluginError::EnvVar(format!("Failed to load LLM_CONFIG from env var: {e}")))?;
    let system_context = match env_var("NB_SYSTEM_CONTEXT") {
        Ok(context) => context,
        Err(err) => {
            log(
                Level::Warn,
                &format!("Failed to retrieve SYSTEM_CONTEXT (llm_utils): {}", err),
            );
            "**System**
            You’re an autonomous AI for Concordance Ind.

            **Context (per request)**
            – Current node
            – Possible destination nodes
            – Available traversal options
            – Graph‑modification tools
            - Light previous traversal context

            **Mission**
            Fulfill each user request by navigating or extending the provided intent graph (DAG).
            Create simple, efficient nodes, generalizable nodes when you do create nodes. Consider how they might be used in the future for similar requests.

            **Tools**
            - `choose_path`: follow an existing edge
            - `connect_to_node`: link two nodes
            - `plan_path`: plan a new path and add it to the graph

            **Rules**
            1. The **root node** defines your central purpose—keep it top of mind.
            2. **Prefer existing paths**: if one fits, call `choose_path`.
            3. **Extend only when needed**: if no path to a fitting node, call `plan_path`.
            4. **Fallback**: if no nodes or paths suffice, call `need_new_node`.
            5. **Stable arguments only**: don’t hard‑code values that change each request (ie. if you want to fetch messages from slack, don't hardcode the number of messages to summarize
            in the tool call node for that).
            6. **Graph coherency**: new nodes/paths must align strictly with user intent.
            7. **Minimize intent drift**: choose the highest‑scent options (scent is tracked externally).
            8. If the user asks you to do something, do not stop at creating a simple sub-intention node, keep going until the entire user request is fulfilled.
            9. If the user issues a correction to you (i.e. 'thats wrong', 'no, thats not what I meant', etc.), choose a path that best backtracks (or use jump_to_root) to where you went wrong and then continue from there.
            ".to_string()
        }
    };
    Ok((llm_config, system_context))
}

pub fn initialize_chat_session(
    req: &NBChatRequest,
    llm_config: &shared::types::LLMConfig,
    system_context: &str,
) -> Result<(ChatSession, String, bool), PluginError> {
    let default_tools = default_tools();
    log(Level::Info, &format!("default tools: {:#?}", default_tools));

    let chat_config = ChatConfig {
        model: llm_config.model_name.clone(),
        temperature: None,
        max_tokens: llm_config.max_tokens,
        top_p: llm_config.top_p,
        top_k: llm_config.top_k,
        tools: Some(default_tools),
        tool_choice: None,
        messages: vec![],
        streaming: Some(streaming_enabled()),
        response_schema: llm_config.response_schema.clone(),
    };

    let client = new_client(&llm_config.base_url, &llm_config.api_key)?;

    let thread_id = req
        .thread_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let thread_file_path = format!("nb/threads/thread-{}.json", thread_id);

    // Ensure directory exists before writing file
    if let Some(parent) = Path::new(&thread_file_path).parent() {
        fs::create_dir_all(parent).map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to create directory for thread file: {}",
                e
            )))
        })?;
    }

    let chat_session;
    let mut is_new_thread = false;

    if Path::new(&thread_file_path).exists() {
        (chat_session, _) =
            load_existing_thread(&thread_file_path, &chat_config, client, system_context)?;
    } else {
        // log(
        //     Level::Warn,
        //     &format!(
        //         "Requested thread with ID {} does not exist, creating new thread (llm_utils)",
        //         thread_id
        //     ),
        // );
        chat_session = ChatSession::new(&chat_config, client, false);
        is_new_thread = true;
        // log(
        //     Level::Info,
        //     &format!("Adding system context (llm_utils): {}", system_context),
        // );

        let msg = Message {
            role: "system".to_string(),
            content: ContentType::Single(MessageContent::Content(system_context.to_string())),
            tool_calls: None,
            tool_call_id: None,
        };
        chat_session.add_message(&msg)?;

        // Initialize an empty ThreadStorage
        let thread_storage = ThreadStorage {
            title: Some("Untitled".to_string()),
            messages: vec![msg],
            current_node: NodeIndex::new(0),
            graph_traces: vec![],
            config: Some(chat_config.clone()),
            tools_in_ctx: false,
        };

        // Save the initial empty thread
        let thread_json = serde_json::to_string_pretty(&thread_storage)
            .map_err(|e| PluginError::Json(format!("Failed to serialize thread: {}", e)))?;
        fs::write(&thread_file_path, thread_json).map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to write thread file from llm_utils: {}",
                e
            )))
        })?;
    }
    Ok((chat_session, thread_id, is_new_thread))
}

fn load_existing_thread(
    thread_file_path: &str,
    chat_config: &ChatConfig,
    client: Client,
    system_context: &str,
) -> Result<(ChatSession, Option<String>), PluginError> {
    let thread_data = fs::read_to_string(thread_file_path).map_err(|e| {
        PluginError::Fs(FsError::Other(format!("Failed to read thread file: {}", e)))
    })?;

    let thread_storage: ThreadStorage = serde_json::from_str(&thread_data)
        .map_err(|e| PluginError::Json(format!("Invalid thread data: {}", e)))?;

    let title = thread_storage.title;
    let chat_session = ChatSession::new(chat_config, client, false);

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

pub fn mcp_tools_split() -> Result<Vec<serde_json::Value>, PluginError> {
    let full_tools_str = mcp_tools()?;
    let full_tools: Vec<serde_json::Value> = serde_json::from_str(&full_tools_str)
        .map_err(|e| PluginError::Json(format!("Invalid tools json: {}", e)))?;
    Ok(full_tools.into_iter().flat_map(split_any_of_tool).collect())
}

pub fn split_any_of_tool(schema: serde_json::Value) -> Vec<serde_json::Value> {
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

pub fn generate_inputs(
    chat_session: &ChatSession,
    mut schema: serde_json::Value,
    context: &str,
    hardcoded_args: &serde_json::Value,
) -> Result<serde_json::Value, PluginError> {
    // Log the schema structure for debugging
    // log(Level::Info, &format!("Using schema: {}", schema));

    schema.as_object_mut().unwrap().remove("examples");

    let schema = serde_json::json!({
        "name": "root",
        "strict": true,
        "schema": schema
    })
    .to_string();
    let schema_chat_session = chat_session.fork_at((chat_session.messages().len() - 1) as u64)?;
    schema_chat_session.set_response_schema(Some(&schema))?;
    schema_chat_session.remove_all_tools()?;
    schema_chat_session.set_tool_choice(None)?;
    schema_chat_session.disable_streaming();
    schema_chat_session.add_message(&Message {
        role: "developer".to_string(),
        content: ContentType::Single(MessageContent::Content(
            format!("Here is the context from the current path traversal that i've builtup thus far: {context}")
        )),
        tool_call_id: None,
        tool_calls: None,
    })?;
    schema_chat_session.add_message(&Message {
        role: "assistant".to_string(),
        content: ContentType::Single(MessageContent::Content(format!(
            "Ok I need to generate the inputs for the tool call. I already have these inputs: {}. I need to generate the remaining inputs to accomplish my original task.",
            hardcoded_args
        ))),
        tool_call_id: None,
        tool_calls: None,
    })?;
    let response = schema_chat_session.send()?;
    let mut extra_json: serde_json::Value = serde_json::from_str(
        response
            .choices
            .first()
            .ok_or_else(|| PluginError::ChatCompletion("No schema response generated".to_string()))?
            .message
            .content
            .as_ref()
            .ok_or_else(|| {
                PluginError::ChatCompletion("No schema response generated".to_string())
            })?,
    )
    .map_err(|_| PluginError::ChatCompletion("Invalid JSON response".to_string()))?;

    merge(&mut extra_json, hardcoded_args);
    Ok(extra_json)
}

fn merge(a: &mut serde_json::Value, b: &serde_json::Value) {
    if let serde_json::Value::Object(a) = a {
        if let serde_json::Value::Object(b) = b {
            for (k, v) in b {
                if v.is_null() {
                    a.remove(k);
                } else {
                    merge(a.entry(k).or_insert(serde_json::Value::Null), v);
                }
            }

            return;
        }
    }

    *a = b.clone();
}
