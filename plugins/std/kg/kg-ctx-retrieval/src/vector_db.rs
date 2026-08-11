use shared::types::EmbeddingConfig;

use crate::models::*;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::{
    host::{log, DbConnection},
    logger::Level,
    open_a_i_like::Client,
    vector_db::SimilaritySearchConfig,
};

/// Client for interacting with a vector database.
/// Provides methods to retrieve node data, edge data, and document chunks.
pub struct VectorDbClient<'a> {
    /// The database connection used for vector operations
    db: &'a DbConnection,
}

impl<'a> VectorDbClient<'a> {
    /// Creates a new VectorDbClient instance.
    ///
    /// # Arguments
    ///
    /// * `db` - A reference to a database connection
    ///
    /// # Returns
    ///
    /// A new VectorDbClient instance.
    pub fn new(db: &'a DbConnection) -> Self {
        Self { db }
    }

    /// Retrieves node data from the vector database based on keyword similarity.
    ///
    /// # Arguments
    ///
    /// * `request` - The context request containing vector database configuration
    /// * `embedding_client` - The client for embedding operations
    /// * `keywords` - The keywords to search for similar nodes
    ///
    /// # Returns
    ///
    /// A Result containing a Vec of VectorDbNodeData if successful, or a PluginError if an error occurs.
    pub fn get_node_data(
        &self,
        request: &ContextRequest,
        embedding_client: &Client,
        embedding_config: &EmbeddingConfig,
        keywords: &str,
    ) -> Result<Vec<VectorDbNodeData>, PluginError> {
        log(
            Level::Info,
            &format!("Querying vector DB for nodes with keywords: {}", keywords),
        );

        let search_config = SimilaritySearchConfig {
            limit: Some(
                request
                    .vector_db_config
                    .similarity_search_config
                    .clone()
                    .unwrap_or_default()
                    .limit
                    .unwrap_or(60),
            ),
            threshold: request
                .vector_db_config
                .similarity_search_config
                .clone()
                .unwrap_or_default()
                .threshold,
            fields_returned: request.vector_db_config.entity_fields.clone(),
            where_clause: request
                .vector_db_config
                .similarity_search_config
                .clone()
                .unwrap_or_default()
                .where_clause
                .clone(),
            include_embeddings: request
                .vector_db_config
                .similarity_search_config
                .clone()
                .unwrap_or_default()
                .include_embeddings,
        };

        let search_results = self
            .db
            .similarity_search(
                &search_config,
                embedding_client,
                &embedding_config.model_name,
                &request.vector_db_config.entity_table_name,
                keywords,
            )?
            .pop()
            .ok_or_else(|| PluginError::KgDb("No search results returned".to_string()))?;

        log(
            Level::Info,
            &format!(
                "Entity search returned {} results",
                search_results.columns.first().map_or(0, |c| c.len())
            ),
        );

        // Create a vector to store the node data
        let mut node_data_list = Vec::new();

        // Get columns and column names from the search results
        let columns = &search_results.columns;
        let column_names = &search_results.column_names;

        // Check if we have sufficient columns
        if columns.len() < 3 || column_names.len() < 3 {
            return Err(PluginError::KgDb(
                "Insufficient columns in search results".to_string(),
            ));
        }

        // Determine the number of rows
        let row_count = columns.first().map_or(0, |c| c.len());

        // Process each row
        for i in 0..row_count {
            let mut properties = std::collections::HashMap::new();
            let mut name = String::new();
            let mut content = String::new();
            let mut source = String::new();

            // Process each column
            for (col_idx, col_name) in column_names.iter().enumerate() {
                if col_idx >= columns.len() {
                    continue; // Skip if column index is out of bounds
                }

                let column_data = &columns[col_idx];
                if i >= column_data.len() {
                    continue; // Skip if row index is out of bounds
                }

                let value = &column_data[i];

                match col_name.as_str() {
                    "name" => name = value.clone(),
                    "content" => content = value.clone(),
                    "doc_source" => source = value.clone(),
                    _ => {
                        // Add any additional columns to properties
                        if !col_name.contains("distance") {
                            // Skip distance column
                            properties.insert(col_name.clone(), value.clone());
                        }
                    }
                }
            }

            // Validate required fields
            if name.is_empty() || content.is_empty() || source.is_empty() {
                return Err(PluginError::KgDb(format!(
                    "Incomplete node data: name={}, content={}, source={}",
                    if name.is_empty() {
                        "missing"
                    } else {
                        "present"
                    },
                    if content.is_empty() {
                        "missing"
                    } else {
                        "present"
                    },
                    if source.is_empty() {
                        "missing"
                    } else {
                        "present"
                    }
                )));
            }

            // Create NodeData and add to the list
            let node = VectorDbNodeData {
                name,
                content,
                source,
                properties,
            };

            node_data_list.push(node);
        }

        // If no nodes were found, return an error
        if node_data_list.is_empty() {
            return Err(PluginError::KgDb(
                "No nodes found in search results".to_string(),
            ));
        }

        Ok(node_data_list)
    }

    /// Retrieves edge data from the vector database based on keyword similarity.
    ///
    /// # Arguments
    ///
    /// * `request` - The context request containing vector database configuration
    /// * `embedding_client` - The client for embedding operations
    /// * `keywords` - The keywords to search for similar edges
    ///
    /// # Returns
    ///
    /// A Result containing a Vec of VectorDbEdgeData if successful, or a PluginError if an error occurs.
    /// The result is sorted by ID and deduplicated to ensure consistent ordering.
    pub fn get_edge_data(
        &self,
        request: &ContextRequest,
        embedding_client: &Client,
        embedding_config: &EmbeddingConfig,
        keywords: &str,
    ) -> Result<Vec<VectorDbEdgeData>, PluginError> {
        log(
            Level::Info,
            &format!("Querying vector DB for edges with keywords: {}", keywords),
        );

        let search_config = SimilaritySearchConfig {
            limit: Some(
                request
                    .vector_db_config
                    .similarity_search_config
                    .clone()
                    .unwrap_or_default()
                    .limit
                    .unwrap_or(60),
            ),
            threshold: request
                .vector_db_config
                .similarity_search_config
                .clone()
                .unwrap_or_default()
                .threshold,
            fields_returned: request.vector_db_config.relationship_fields.clone(),
            where_clause: request
                .vector_db_config
                .similarity_search_config
                .clone()
                .unwrap_or_default()
                .where_clause
                .clone(),
            include_embeddings: request
                .vector_db_config
                .similarity_search_config
                .clone()
                .unwrap_or_default()
                .include_embeddings,
        };

        let search_results = self
            .db
            .similarity_search(
                &search_config,
                embedding_client,
                &embedding_config.model_name,
                &request.vector_db_config.relationship_table_name,
                keywords,
            )?
            .pop()
            .ok_or_else(|| PluginError::KgDb("No search results returned".to_string()))?;

        log(
            Level::Info,
            &format!(
                "Edge search returned {} results",
                search_results.columns.first().map_or(0, |c| c.len())
            ),
        );

        // Create a vector to store the edge data
        let mut edge_data_list = Vec::new();

        // Get columns and column names from the search results
        let columns = &search_results.columns;
        let column_names = &search_results.column_names;

        // Check if we have sufficient columns
        if columns.len() < 2 || column_names.len() < 2 {
            return Err(PluginError::KgDb(
                "Insufficient columns in search results".to_string(),
            ));
        }

        // Determine the number of rows
        let row_count = columns.first().map_or(0, |c| c.len());

        // Process each row
        for i in 0..row_count {
            let mut properties = std::collections::HashMap::new();
            let mut id = String::new();
            let mut source = String::new();
            let mut target = String::new();
            let mut keywords = String::new();
            let mut strength = String::new();
            let mut description = String::new();
            let mut doc_source = String::new();

            // Process each column
            for (col_idx, col_name) in column_names.iter().enumerate() {
                if col_idx >= columns.len() {
                    continue; // Skip if column index is out of bounds
                }

                let column_data = &columns[col_idx];
                if i >= column_data.len() {
                    continue; // Skip if row index is out of bounds
                }

                let value = &column_data[i];

                match col_name.as_str() {
                    "id" => id = value.clone(),
                    "source" => source = value.clone(),
                    "target" => target = value.clone(),
                    "keywords" => keywords = value.clone(),
                    "strength" => strength = value.clone(),
                    "description" => description = value.clone(),
                    "doc_source" => doc_source = value.clone(),
                    _ => {
                        // Add any additional columns to properties
                        if !col_name.contains("distance") {
                            // Skip distance column
                            properties.insert(col_name.clone(), value.clone());
                        }
                    }
                }
            }

            // Create EdgeData and add to the list
            let edge = VectorDbEdgeData {
                id,
                source,
                target,
                keywords,
                strength,
                description,
                doc_source,
                properties,
            };

            edge_data_list.push(edge);
        }

        // If no edges were found, return an error
        if edge_data_list.is_empty() {
            return Err(PluginError::KgDb(
                "No edges found in search results".to_string(),
            ));
        }

        // Sort edge data by ID first for consistent ordering
        edge_data_list.sort_by(|a, b| a.id.cmp(&b.id));

        // Remove duplicates based on ID
        let mut seen_ids = std::collections::HashSet::new();
        edge_data_list.retain(|edge| seen_ids.insert(edge.id.clone()));

        Ok(edge_data_list)
    }

    /// Retrieves document chunks from the database based on source identifiers.
    ///
    /// # Arguments
    ///
    /// * `request` - The context request containing configuration options
    /// * `indexed_doc_sources` - Vector of (index, source identifier) tuples
    /// * `doc_source_references` - Vector of document source references for sorting
    ///
    /// # Returns
    ///
    /// A Result containing a tuple of:
    /// - A Vec of document chunk contents
    /// - A Vec of corresponding document chunk source identifiers
    ///
    /// Results are sorted, deduplicated, and truncated to fit within token limits.
    pub fn get_document_chunks(
        &self,
        request: &ContextRequest,
        indexed_doc_sources: Vec<(usize, String)>,
        doc_source_references: Vec<String>,
    ) -> Result<(Vec<String>, Vec<String>), PluginError> {
        log(
            Level::Info,
            &format!("Retrieving {} document chunks", indexed_doc_sources.len()),
        );

        // get all chunks associated with the core nodes
        let mut results: Vec<(usize, String, String)> = vec![];
        for (idx, doc_source) in indexed_doc_sources {
            // Apply chunk prefix filter if set
            if let Some(chunk_prefix) = &request.chunk_prefix {
                if !doc_source.starts_with(chunk_prefix) {
                    continue;
                }
            }

            // Get the chunk content from the database
            let res =
                self.db
                    .get_row_by_id("chunks", "id", &doc_source, &["content".to_string()][..])?;

            if let Some(content) = res {
                let chunk: serde_json::Value =
                    serde_json::from_str(&content).map_err(|e| PluginError::Json(e.to_string()))?;

                if let Some(content_value) = chunk.as_object().and_then(|obj| obj.get("content")) {
                    if let Some(content_str) = content_value.as_str() {
                        results.push((idx, doc_source, content_str.to_string()));
                    }
                }
            }
        }

        // Sort results by idx first (ascending) and then by connection count (descending) when idx is the same
        results.sort_by(|a, b| {
            let (idx_a, doc_source_a, _) = a;
            let (idx_b, doc_source_b, _) = b;

            // First compare idx (ascending)
            match idx_a.cmp(idx_b) {
                std::cmp::Ordering::Equal => {
                    // If idx is the same, sort by connections (descending)
                    let connections_a = doc_source_references
                        .iter()
                        .filter(|hop_id| *hop_id == doc_source_a)
                        .count();

                    let connections_b = doc_source_references
                        .iter()
                        .filter(|hop_id| *hop_id == doc_source_b)
                        .count();

                    connections_b.cmp(&connections_a) // Descending order for connections
                }
                other => other, // Use the idx comparison result
            }
        });

        // Deduplicate chunks while preserving order
        let mut chunks = vec![];
        let mut chunk_srcs = vec![];
        results.into_iter().for_each(|(_, src, chunk)| {
            if !chunks.contains(&chunk) {
                chunks.push(chunk);
                chunk_srcs.push(src);
            }
        });

        // Truncate to fit max token limit
        let max_tokens = request.context_max_tokens.unwrap_or(2000);
        let doc_chunks = crate::utils::truncate_by_token_size(chunks, max_tokens)
            .map_err(PluginError::Unexpected)?;

        let chunk_srcs = chunk_srcs
            .into_iter()
            .take(doc_chunks.len())
            .collect::<Vec<String>>();

        log(
            Level::Info,
            &format!("Returning {} document chunks", doc_chunks.len()),
        );

        Ok((doc_chunks, chunk_srcs))
    }
}
