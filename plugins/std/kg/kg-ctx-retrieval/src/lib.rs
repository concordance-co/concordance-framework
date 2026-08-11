mod context_builder;
mod keyword_extractor;
mod kg_client;
mod models;
mod strategies;
mod utils;
mod vector_db;

// Construct the injector plugin interface
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

use crate::plugin::injector::{
    host::{connect_db, log, new_client, update_status},
    logger::Level,
};
use exports::plugin::injector::guest::{Guest, GuestJsonToJson, Metadata, PluginError, PluginKind};
use models::*;
use shared::{
    inlined_schema_for,
    types::{EmbeddingConfig, LLMConfig},
    TryFromEnvVar,
};
use strategies::StrategyFactory;

struct ContextRetrievalPlugin;

impl Guest for ContextRetrievalPlugin {
    type JsonToJson = ContextRetrieval;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "KG Context Retrieval".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock elmore".to_string(),
            description: "A plugin that retrieves knowledge graph contexts for queries".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![
                ("llm_config".to_string(), "LLM_CONFIG".to_string()),
                (
                    "embedding_config".to_string(),
                    "EMBEDDING_CONFIG".to_string(),
                ),
            ],
            input_schema: serde_json::to_string(&inlined_schema_for!(ContextRequest)).unwrap(),
            default_input: serde_json::to_string(&ContextRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(ContextResponse)).unwrap(),
        }
    }
}

pub struct ContextRetrieval {
    pub last_keywords: Option<KeywordInfo>,
    pub keyword_extractor: keyword_extractor::KeywordExtractor,
}

impl GuestJsonToJson for ContextRetrieval {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Parse input
        update_status("Starting context retrieval");
        let request = match serde_json::from_str::<ContextRequest>(&input) {
            Ok(req) => req,
            Err(e) => {
                return Err(PluginError::Json(format!("Invalid input: {}", e)));
            }
        };

        let embedding_config = match request.embedding_config {
            Some(ref config) => config.clone(),
            None => EmbeddingConfig::try_from_env_var("EMBEDDING_CONFIG").map_err(|e| {
                PluginError::EnvVar(format!("Failed to load EMBEDDING_CONFIG: {}", e))
            })?,
        };
        let llm_config = match request.llm_config {
            Some(ref config) => config.clone(),
            None => LLMConfig::try_from_env_var("LLM_CONFIG")
                .map_err(|e| PluginError::EnvVar(format!("Failed to load LLM_CONFIG: {}", e)))?,
        };

        log(Level::Info, &format!("Processing query: {}", request.query));

        // 1. Extract keywords from the query
        update_status("Extracting keywords...");
        let (high_level_keywords, low_level_keywords) = self
            .keyword_extractor
            .extract_keywords(&request, &llm_config)?;

        if high_level_keywords.is_empty() && low_level_keywords.is_empty() {
            log(
                Level::Warn,
                "Both high-level and low-level keywords are empty",
            );
            return Ok(serde_json::to_string(&ContextResponse {
                kg_context: None,
                keywords: None,
            })
            .unwrap());
        }

        // Store keywords for response
        let keyword_info = KeywordInfo {
            high_level: high_level_keywords.clone(),
            low_level: low_level_keywords.clone(),
            mode: format!("{:?}", request.mode.unwrap_or_default()),
        };

        // 2. Set up clients and connections
        update_status("Connecting to databases...");
        let embedding_client = new_client(&embedding_config.base_url, &embedding_config.api_key)?;

        let db = connect_db(&request.vector_db_config.db_path)?;

        let kg_auth = serde_json::json!({
            "uri": "http://localhost:7474",
            "username": "neo4j",
            "password": "password",
            "database": "neo4j"
        });

        // 3. Create and execute the appropriate strategy based on mode
        update_status("Retrieving knowledge graph context...");
        let mode = request.mode.unwrap_or_default();
        let strategy = StrategyFactory::create_strategy(mode, &db);

        let context_data = strategy.execute(
            &request,
            &embedding_client,
            &embedding_config,
            &db,
            &kg_auth,
            &high_level_keywords,
            &low_level_keywords,
        )?;

        // 4. Prepare response
        update_status("Preparing response...");
        let response =
            ContextResponse {
                kg_context: Some(serde_json::to_string(&context_data).map_err(|e| {
                    PluginError::Json(format!("Failed to serialize response: {}", e))
                })?),
                keywords: Some(keyword_info),
            };

        // Serialize and return
        match serde_json::to_string(&response) {
            Ok(json) => Ok(json),
            Err(e) => Err(PluginError::Json(format!(
                "Failed to serialize response: {}",
                e
            ))),
        }
    }

    fn new() -> Self {
        Self {
            last_keywords: None,
            keyword_extractor: keyword_extractor::KeywordExtractor::new(),
        }
    }
}

export!(ContextRetrievalPlugin);
