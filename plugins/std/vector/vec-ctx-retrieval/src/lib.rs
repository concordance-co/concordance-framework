// Construct the injector plugin interface
wit_bindgen::generate!({
    world: "injector",
    path: "../../../../wit",
    generate_all,
});
use crate::plugin::injector::host::new_client;
use crate::plugin::injector::vector_db::SimilaritySearchConfig;
use exports::plugin::injector::guest::{Guest, GuestJsonToJson, Metadata, PluginError, PluginKind};

// host capabilities
use plugin::injector::host::{connect_db, log, update_status};
use plugin::injector::logger::Level;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::types::{EmbeddingConfig, SimSearchConfig};
use shared::{inlined_schema_for, TryFromEnvVar};
use tiktoken_rs::o200k_base;

// Vector DB related structures
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct VectorDbConfig {
    pub db_path: String,
    pub table_name: String,
    pub similarity_search_config: SimSearchConfig,
}

impl Default for VectorDbConfig {
    fn default() -> Self {
        Self {
            db_path: "".to_string(),
            table_name: "".to_string(),
            similarity_search_config: SimSearchConfig::default(),
        }
    }
}

// Input structure for the plugin
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ContextRequest {
    pub query: String,
    pub chunk_prefix: Option<String>,
    /// Defaults to 10
    pub max_chunks: Option<usize>,
    /// For each chunk, limit the number of tokens returned
    pub per_chunk_max_tokens: Option<usize>,
    pub embedding_config: Option<EmbeddingConfig>,
    pub vector_db_config: VectorDbConfig,
}

// Output structure
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ContextResponse {
    pub vector_context: String,
}

struct ContextRetrievalPlugin;

impl Guest for ContextRetrievalPlugin {
    type JsonToJson = ContextRetrieval;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "Vector Context Retrieval".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock elmore".to_string(),
            description: "A plugin that retrieves vector contexts for queries".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![(
                "embedding_config".to_string(),
                "EMBEDDING_CONFIG".to_string(),
            )],
            input_schema: serde_json::to_string(&inlined_schema_for!(ContextRequest)).unwrap(),
            default_input: serde_json::to_string(&ContextRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(ContextResponse)).unwrap(),
        }
    }
}

struct ContextRetrieval;

impl GuestJsonToJson for ContextRetrieval {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Parse input
        update_status("Starting vector database context retrieval");
        let request = match serde_json::from_str::<ContextRequest>(&input) {
            Ok(req) => req,
            Err(e) => {
                return Err(PluginError::Json(format!("Invalid input: {}", e)));
            }
        };

        log(Level::Info, &format!("Processing query: {}", request.query));

        let embedding_config = match request.embedding_config {
            Some(ref config) => config.clone(),
            None => EmbeddingConfig::try_from_env_var("EMBEDDING_CONFIG").map_err(|e| {
                PluginError::EnvVar(format!("Failed to load EMBEDDING_CONFIG: {}", e))
            })?,
        };

        // Results placeholder
        let mut response = ContextResponse {
            vector_context: Default::default(),
        };

        response.vector_context = self
            .get_vector_context(&request, &embedding_config)
            .map_err(PluginError::VectorDb)?;

        update_status("Finished vector database context retrieval");
        // Serialize and return
        Ok(serde_json::to_string(&response).unwrap())
    }

    fn new() -> Self {
        Self {}
    }
}

impl ContextRetrieval {
    fn get_vector_context(
        &self,
        request: &ContextRequest,
        embedding_config: &EmbeddingConfig,
    ) -> Result<String, String> {
        log(Level::Info, "Getting vector context from vector database");
        // Reduce top_k for hybrid mode
        let top_k = request.max_chunks.unwrap_or(10);

        log(Level::Info, &format!("Vector search with top_k={}", top_k));

        // Set up the vector db config
        let vector_config = request.vector_db_config.clone();
        let table_name = vector_config.table_name;

        // Initialize the vector DB client
        let db = match connect_db(&vector_config.db_path) {
            Ok(db) => db,
            Err(e) => return Err(format!("Failed to initialize vector database: {:?}", e)),
        };

        // Initialize embedding client
        let base_url = embedding_config.base_url.clone();
        let api_key = embedding_config.api_key.clone();

        let embedding_client = match new_client(&base_url, &api_key) {
            Ok(client) => client,
            Err(e) => return Err(format!("Failed to create embedding client: {:?}", e)),
        };

        // Set up the search configuration
        let search_config = SimilaritySearchConfig {
            limit: Some(top_k as u32),
            threshold: vector_config.similarity_search_config.threshold,
            fields_returned: vector_config.similarity_search_config.fields_returns,
            where_clause: vector_config.similarity_search_config.where_clause,
            include_embeddings: vector_config.similarity_search_config.include_embeddings,
        };

        // Perform the similarity search
        update_status("Performing vector database similarity search");
        let search_results = match db.similarity_search(
            &search_config,
            &embedding_client,
            &embedding_config.model_name,
            &table_name,
            &request.query,
        ) {
            Ok(results) => results,
            Err(e) => return Err(format!("Vector search failed: {:?}", e)),
        };
        // Convert results to a value we can work with - raw handling without serialization
        log(
            Level::Info,
            &format!(
                "Vector search returned {} results -- {search_results:#?}",
                search_results.len()
            ),
        );

        // Extract all content values from the search results
        let mut contents = Vec::new();

        for result in &search_results {
            // Transform columns into rows and stringify the row to add to contents

            // Get number of rows based on first column's length (if available)
            if let Some(first_column) = result.columns.first() {
                let row_count = first_column.len();

                // Process each row
                'row: for row_idx in 0..row_count {
                    let mut row_values = Vec::new();

                    // Collect values from each column for this row
                    for (i, column) in result.columns.iter().enumerate() {
                        // if we have a chunk prefix, and the column name is "id",
                        // ensure that the id starts with the prefix, otherwise skip the row
                        if let Some(chunk_prefix) = &request.chunk_prefix {
                            if result.column_names[i] == "id"
                                && !column[row_idx].starts_with(chunk_prefix)
                            {
                                continue 'row;
                            }
                        }

                        if row_idx < column.len() {
                            row_values.push(column[row_idx].clone());
                        }
                    }

                    // Join row values into a string
                    contents.push(row_values.join(", "));
                }
            }
        }

        if contents.is_empty() {
            return Ok(Default::default());
        }

        let chunks = contents.len();

        // Truncate chunks if they exceed max token size
        let max_token_size = request.per_chunk_max_tokens.unwrap_or(2000);
        let truncated_chunks = truncate_by_token_size(contents, max_token_size)?;

        log(
            Level::Info,
            &format!(
                "Truncated chunks from {} to {} (max tokens:{})",
                chunks,
                truncated_chunks.len(),
                max_token_size
            ),
        );

        // Join the chunks with separators
        let vector_context = truncated_chunks.join("\n--New Chunk--\n");
        Ok(vector_context)
    }
}

// Truncate chunks by token size using o200k_base tokenizer
fn truncate_by_token_size(chunks: Vec<String>, max_tokens: usize) -> Result<Vec<String>, String> {
    let bpe = o200k_base().unwrap();
    let mut result = Vec::new();

    for chunk in chunks {
        result.push(
            match bpe
                .split_by_token_iter(&chunk, true)
                .take(max_tokens)
                .collect::<Result<Vec<String>, _>>()
            {
                Ok(r) => r.join(""),
                Err(e) => return Err(format!("Unable to byte pair encode chunk: {e}")),
            },
        );
    }

    Ok(result)
}

export!(ContextRetrievalPlugin);
