use crate::plugin::injector::open_a_i_like::ChatConfig;
use crate::plugin::injector::open_a_i_like::Message;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::types::LLMConfig;
use shared::types::SlashCommand;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ChatRequest {
    pub message: String,
    pub slash_commands: Option<Vec<SlashCommand>>,
    pub system_context: Option<String>,
    pub thread_id: Option<String>,
    pub llm_config: Option<LLMConfig>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ChatResponse {
    pub response: String,
    pub thread_id: String,
}

// Thread storage format
#[derive(Debug, Deserialize, Serialize)]
pub struct ThreadStorage {
    pub title: Option<String>,
    pub messages: Vec<Message>,
    pub config: Option<ChatConfig>,
}

// Thread summary format
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ThreadSummary {
    pub threads: Vec<ThreadInfo>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ThreadInfo {
    pub id: String,
    pub title: Option<String>,
    pub last_message_timestamp: u64,
}
