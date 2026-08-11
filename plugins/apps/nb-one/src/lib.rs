mod graph_utils;
mod llm_utils;
mod message_utils;
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

use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};
use crate::graph_utils::*;
use crate::message_utils::*;
use crate::plugin::injector::host::log;
use crate::plugin::injector::host::Level;

use petgraph::dot::Dot;
// use llm_utils::save_thread_data;
use plugin::injector::host::{new_client, streaming_enabled, SseEvent};
use shared::{inlined_schema_for, types::EmbeddingConfig, TryFromEnvVar};

pub struct NBMindPlugin;

impl Guest for NBMindPlugin {
    type JsonToJson = NBMind;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "NB Mind".to_string(),
            version: "0.1.0".to_string(),
            author: "Marshall Vyletel".to_string(),
            description: "Autonomous Mind Experiment".to_string(),
            env_var_support: vec![
                ("llm_config".to_string(), "LLM_CONFIG".to_string()),
                ("system_context".to_string(), "SYSTEM_CONTEXT".to_string()),
            ],
            kind: PluginKind::ChatApp,
            input_schema: serde_json::to_string(&inlined_schema_for!(NBChatRequest)).unwrap(),
            default_input: serde_json::to_string(&NBChatRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(NBChatResponse)).unwrap(),
        }
    }
}

pub struct NBMind;

impl GuestJsonToJson for NBMind {
    fn new() -> Self {
        Self {}
    }
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Parse input JSON
        let chat_request: NBChatRequest = serde_json::from_str(&input)
            .map_err(|e| PluginError::Json(format!("Failed to parse input: {}", e)))?;

        // Init / Load DAG
        let mut dag = init_graph()?;

        let dag_json = serde_json::to_string_pretty(&dag)
            .map_err(|e| PluginError::Json(format!("Failed to stringify dag: {}", e)))?;
        log(
            Level::Info,
            &format!(
                "CURRENT DAG: {}\n\nAS MERMAID: {:?}",
                dag_json,
                Dot::new(&dag)
            ),
        );

        // Init / Load Chat Session
        let (llm_config, system_context) = llm_utils::setup_config()?;
        let (mut chat_session, thread_id, is_new_thread) =
            llm_utils::initialize_chat_session(&chat_request, &llm_config, &system_context)?;

        if is_new_thread && streaming_enabled() {
            let event = SseEvent::new();
            event.set_data(&format!("{{\"set_thread_id\": \"{}\"}}", thread_id))?;
            event.send()?;
        }

        // Get Node Options
        let embedding_config = EmbeddingConfig::try_from_env_var("EMBEDDING_CONFIG")
            .map_err(|e| PluginError::EnvVar(format!("Failed to load embedding config: {}", e)))?;
        let embedding_client = new_client(&embedding_config.base_url, &embedding_config.api_key)?;
        let node_options = graph_utils::embedding_options(
            &embedding_client,
            &embedding_config.model_name,
            "nb_data",
            &chat_request.message,
        )?;

        log(
            Level::Info,
            &format!("Embedding options (lib): {:#?}", node_options),
        );

        if node_options.is_empty() {
            let _ = chat_session.remove_tool("connect_to_node")?;
            let _ = chat_session.remove_tool("choose_path")?;
        }

        // Process user message
        let response = process_message(
            &chat_request,
            &mut chat_session,
            node_options,
            &thread_id,
            &mut dag,
        )?;

        if streaming_enabled() {
            let event = SseEvent::new();
            event.set_data(&format!("end result: {:?}", response))?;
            event.send()?;
        }

        serde_json::to_string(&response)
            .map_err(|e| PluginError::Json(format!("Failed to serialize output: {}", e)))
    }
}

export!(NBMindPlugin);
