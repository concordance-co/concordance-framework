mod chat_session;
mod message_utils;
mod thread_storage;
mod types;

wit_bindgen::generate!({
    world: "injector",
    path: "../../../wit",
    additional_derives: [
        serde::Serialize,
        serde::Deserialize,
        schemars::JsonSchema,
        Clone,
        PartialEq,
    ],
});

use crate::chat_session::*;
use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};
use crate::plugin::injector::host::Level;
use crate::plugin::injector::host::{call_plugin, log};
use crate::plugin::injector::open_a_i_like::{ContentType, Message, MessageContent};

use crate::thread_storage::*;
use crate::types::*;
use plugin::injector::host::{streaming_enabled, SseEvent};
use shared::inlined_schema_for;
use shared::types::SlashCommandOutput;

pub struct OpenAIChatPlugin;

impl Guest for OpenAIChatPlugin {
    type JsonToJson = OpenAIChat;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "EXEA".to_string(),
            version: "0.1.0".to_string(),
            author: "Your Name".to_string(),
            description: "EXEA: An executive admin experiment".to_string(),
            env_var_support: vec![
                ("llm_config".to_string(), "LLM_CONFIG".to_string()),
                ("system_context".to_string(), "SYSTEM_CONTEXT".to_string()),
            ],
            kind: PluginKind::ChatApp,
            input_schema: serde_json::to_string(&inlined_schema_for!(ChatRequest)).unwrap(),
            default_input: serde_json::to_string(&ChatRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(ChatResponse)).unwrap(),
        }
    }
}

pub struct OpenAIChat;

impl GuestJsonToJson for OpenAIChat {
    fn work(&self, input: String) -> Result<String, PluginError> {
        log(Level::Info, &format!("Input: {}", input));
        // Parse input JSON
        let req: ChatRequest = parse_chat_request(&input)?;

        // Setup environment and configuration
        let (llm_config, system_context, had_memories) = setup_config(&req)?;

        // Initialize or load chat session
        let (mut chat_session, thread_id, is_new_thread, title) =
            initialize_chat_session(&req, &llm_config, &system_context, had_memories)?;

        if let Some(slash_commands) = req.slash_commands {
            for slash_command in slash_commands {
                log(
                    Level::Info,
                    &format!("Calling slash command: {}", slash_command.command),
                );
                let args = serde_json::to_string(&slash_command.args).unwrap();
                let slash_res = serde_json::from_str::<SlashCommandOutput>(
                    &call_plugin(slash_command.command.as_str(), args.as_str()).map_err(|err| {
                        log(
                            Level::Error,
                            &format!("Error calling slash command: {}", err),
                        );
                        err
                    })?,
                )
                .map_err(|err| PluginError::Json(err.to_string()))?;
                log(
                    Level::Info,
                    &format!("Added slash command result: {}", slash_res.output),
                );
                chat_session.add_message(&Message {
                    role: "user".to_string(),
                    content: ContentType::Single(MessageContent::Content(slash_res.output)),
                    tool_calls: None,
                    tool_call_id: None,
                })?;
            }
        }

        if is_new_thread && streaming_enabled() {
            let event = SseEvent::new();
            event.set_data(&format!("{{\"set_thread_id\": \"{}\"}}", thread_id))?;
            event.send()?;
        }

        // Log the thread ID for debugging purposes
        log(
            Level::Info,
            &format!(
                "Processing message for thread ID: {}, new: {}",
                thread_id, is_new_thread
            ),
        );

        // Process the user message
        let response_text = process_user_message(&mut chat_session, &req.message)?;

        let new_title = chat_session.session_title();

        if new_title != title {
            if let Some(ref new_title) = new_title {
                let event = SseEvent::new();
                event.set_data(&format!("{{\"set_title\": \"{}\"}}", new_title))?;
                event.send()?;
            }
        }

        // Save thread data
        save_thread_data(&chat_session, &thread_id, new_title)?;

        // Create and return response
        let chat_response = ChatResponse {
            response: response_text,
            thread_id,
        };

        serde_json::to_string(&chat_response).map_err(|e| PluginError::Json(e.to_string()))
    }

    fn new() -> Self {
        Self {}
    }
}

export!(OpenAIChatPlugin);
