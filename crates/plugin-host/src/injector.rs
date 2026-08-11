//! This module defines the bindings and conversions for the plugin injector.
//!
//! It generates bindings for the WebAssembly Interface Type (WIT) world "injector",
//! and provides utility functions for working with plugin metadata.
#![allow(clippy::derivable_impls)]

wasmtime::component::bindgen!({
    world: "injector",
    path: "../../wit",
    async: true,
    additional_derives: [
        serde::Serialize,
        serde::Deserialize,
        schemars::JsonSchema,
        Clone,
        PartialEq,
    ],
    with: {
        "plugin:injector/vector-db/db-connection": crate::host::DbConn,
        "plugin:injector/open-a-i-like/open-a-i-config": crate::host::OpenAIConfig,
        "plugin:injector/open-a-i-like/chat-session": crate::host::ChatSession,
        "plugin:injector/open-a-i-like/client": crate::host::Client,
        "plugin:injector/host/sse-event": axum::response::sse::Event,
    }
});

use crate::injector::host::MetaToolInfo;
pub use crate::injector::plugin::injector::{
    env, error, host, http, logger, markdown_converter, open_a_i_like, vector_db,
};
pub use exports::plugin::injector::guest::Metadata;
use serde::{Deserialize, Serialize};

use self::open_a_i_like::CompletionTokenDetails;

impl Default for open_a_i_like::Usage {
    fn default() -> Self {
        Self {
            completion_tokens: 0,
            prompt_tokens: 0,
            total_tokens: 0,
            completion_token_details: None,
            prompt_token_details: None,
        }
    }
}

impl From<async_openai::types::CompletionUsage> for open_a_i_like::Usage {
    fn from(usage: async_openai::types::CompletionUsage) -> Self {
        Self {
            completion_tokens: usage.completion_tokens as u64,
            prompt_tokens: usage.prompt_tokens as u64,
            total_tokens: usage.total_tokens as u64,
            prompt_token_details: None,
            completion_token_details: None,
        }
    }
}

impl From<async_openai::types::PromptTokensDetails> for open_a_i_like::PromptTokenDetails {
    fn from(details: async_openai::types::PromptTokensDetails) -> Self {
        Self {
            audio_tokens: details.audio_tokens.map(|t| t as u64).unwrap_or_default(),
            cached_tokens: details.cached_tokens.map(|t| t as u64).unwrap_or_default(),
        }
    }
}

impl From<async_openai::types::CompletionTokensDetails> for CompletionTokenDetails {
    fn from(details: async_openai::types::CompletionTokensDetails) -> Self {
        Self {
            accepted_prediction_tokens: details
                .accepted_prediction_tokens
                .map(|t| t as u64)
                .unwrap_or_default(),
            audio_tokens: details.audio_tokens.map(|t| t as u64).unwrap_or_default(),
            reasoning_tokens: details
                .reasoning_tokens
                .map(|t| t as u64)
                .unwrap_or_default(),
            rejected_prediction_tokens: details
                .rejected_prediction_tokens
                .map(|t| t as u64)
                .unwrap_or_default(),
        }
    }
}

/// Converts plugin metadata to a unique identifier string.
///
/// Creates an ID by combining the lowercase, hyphenated name with the version.
///
/// # Arguments
///
/// * `metadata` - The plugin metadata containing name and version
///
/// # Returns
///
/// A formatted string in the format `{name}-{version}` where name has spaces replaced with hyphens
pub fn metadata_to_id(metadata: &Metadata) -> String {
    metadata.name.to_lowercase().replace(' ', "-")
}

/// Represents the schema for a tool that can be used with LLM function calling.
///
/// This structure follows the OpenAI function calling format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// The type of the tool, typically "function"
    #[serde(rename = "type")]
    pub ty: String,
    /// The function definition
    pub function: Function,
}

/// Represents a function that can be called by an LLM.
///
/// Contains the metadata needed to describe the function to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// The name of the function
    pub name: String,
    /// A description of what the function does
    pub description: String,
    /// The parameters the function accepts, as a JSON schema
    pub parameters: serde_json::Value,
}

/// Converts plugin metadata to a tool schema for LLM function calling.
///
/// # Arguments
///
/// * `metadata` - The plugin metadata containing name, description and input schema
///
/// # Returns
///
/// A ToolSchema object that can be used with LLM function calling APIs
pub fn metadata_to_tool_schema(metadata: &Metadata) -> ToolSchema {
    let name = metadata_to_id(metadata);
    ToolSchema {
        ty: "function".to_string(),
        function: Function {
            name,
            description: metadata.description.clone(),
            parameters: serde_json::from_str(&metadata.input_schema).unwrap(),
        },
    }
}

/// Converts plugin metadata to a meta tool info for LLM function calling.
///
/// # Arguments
///
/// * `metadata` - The plugin metadata containing name and description
///
/// # Returns
///
/// A MetaToolInfo object that can be used with LLM function calling APIs
pub fn metadata_to_meta_tool(metadata: &Metadata) -> MetaToolInfo {
    let name = metadata_to_id(metadata);
    MetaToolInfo {
        name,
        description: metadata.description.clone(),
    }
}
