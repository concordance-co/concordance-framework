use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct LLMConfig {
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f64>,
    pub top_k: Option<u32>,
    pub tools: Option<Vec<String>>,
    pub response_schema: Option<String>,
}

pub type EmbeddingConfig = LLMConfig;

#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SimSearchConfig {
    pub limit: Option<u32>,
    pub threshold: Option<f64>,
    pub include_embeddings: Option<bool>,
    pub fields_returns: Vec<String>,
    pub where_clause: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SlashCommand {
    pub command: String,
    pub args: SlashCommandInput,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SlashCommandInput {
    pub input: String,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SlashCommandOutput {
    pub output: String,
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
