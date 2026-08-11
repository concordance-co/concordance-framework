wit_bindgen::generate!({
    world: "injector",
    path: "../../../../wit",
    additional_derives: [
        serde::Serialize,
        serde::Deserialize,
        Clone,
        PartialEq,
    ],
});
use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};
use crate::plugin::injector::{host::new_client, open_a_i_like::EmbeddingInput};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::inlined_schema_for;
use shared::types::EmbeddingConfig;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct EmbeddingRequest {
    /// The configuration for the embedding model.
    pub embedding_config: EmbeddingConfig,
    /// The strings to embed.
    pub strs: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Resp {
    /// The vector embeddings for the input strings.
    pub result: Vec<Vec<f32>>,
}

pub struct EmbedderPlugin;

impl Guest for EmbedderPlugin {
    type JsonToJson = Embedder;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Vector Embedder".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "Converts a list of strings into a list of vector embeddings.".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&inlined_schema_for!(EmbeddingRequest)).unwrap(),
            default_input: serde_json::to_string(&EmbeddingRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}
pub struct Embedder;
impl GuestJsonToJson for Embedder {
    fn work(&self, input: String) -> Result<String, PluginError> {
        let req: EmbeddingRequest =
            serde_json::from_str(&input).map_err(|e| PluginError::Json(e.to_string()))?;
        let client = new_client(
            &req.embedding_config.base_url,
            &req.embedding_config.api_key,
        )?;
        let embeddings = client.embeddings_create_simple(
            &req.embedding_config.model_name,
            &EmbeddingInput::StrArray(req.strs),
        )?;

        serde_json::to_string(&Resp { result: embeddings })
            .map_err(|e| PluginError::Json(e.to_string()))
    }

    fn new() -> Self {
        Self {}
    }
}

export!(EmbedderPlugin);
