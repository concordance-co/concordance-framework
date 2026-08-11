use crate::models::*;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::{host::log, logger::Level};

/// A builder struct for constructing context for knowledge graph entities, edges, and documents.
pub struct ContextBuilder {}

impl ContextBuilder {
    /// Creates a new instance of ContextBuilder.
    ///
    /// # Returns
    ///
    /// A new ContextBuilder instance.
    pub fn new() -> Self {
        Self {}
    }

    /// Constructs formatted context for entities in the knowledge graph.
    ///
    /// # Arguments
    ///
    /// * `entity_chunks` - Vector of entity descriptions as strings
    /// * `node_data` - Slice of KgNodeData objects containing entity information
    ///
    /// # Returns
    ///
    /// A formatted string containing entity context with fields including index, entity ID,
    /// entity type, description, and degree.
    pub fn construct_entity_context(
        &self,
        entity_chunks: Vec<String>,
        node_data: &[KgNodeData],
    ) -> String {
        log(
            Level::Info,
            &format!(
                "Building entity context from {} entries",
                entity_chunks.len()
            ),
        );

        entity_chunks
            .iter()
            .zip(node_data.iter())
            .enumerate()
            .map(|(index, (description, entity))| {
                format!(
                    "{index}, {}, {}, {}, {}",
                    entity.entity_id,
                    entity.entity_type,
                    description,
                    entity
                        .properties
                        .get("degree")
                        .map(|v| v.as_u64().unwrap_or(0))
                        .unwrap_or(0),
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Constructs formatted context for edges in the knowledge graph.
    ///
    /// # Arguments
    ///
    /// * `edge_chunks` - Vector of edge descriptions as strings
    /// * `edge_info` - Vector of tuples containing RelationshipEdge and rank
    ///
    /// # Returns
    ///
    /// A formatted string containing edge context with fields including index, edge ID,
    /// source entity ID, target entity ID, description, keywords, weight, and rank.
    pub fn construct_edge_context(
        &self,
        edge_chunks: Vec<String>,
        edge_info: Vec<(RelationshipEdge, i32)>,
    ) -> String {
        log(
            Level::Info,
            &format!("Building edge context from {} entries", edge_chunks.len()),
        );

        edge_chunks
            .iter()
            .zip(edge_info)
            .enumerate()
            .map(|(index, (description, (edge, rank)))| {
                format!(
                    "{index}, {}, {}, {}, {}, {}, {}, {}",
                    edge.id,
                    edge.source_entity_id,
                    edge.target_entity_id,
                    description,
                    edge.keywords,
                    edge.weight,
                    rank
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Constructs formatted context for document chunks.
    ///
    /// # Arguments
    ///
    /// * `doc_chunks` - Vector of document chunk contents
    /// * `chunk_sources` - Vector of document chunk sources/identifiers
    ///
    /// # Returns
    ///
    /// A formatted string containing document context with fields including index, source, and chunk content.
    pub fn construct_document_context(
        &self,
        doc_chunks: Vec<String>,
        chunk_sources: Vec<String>,
    ) -> String {
        log(
            Level::Info,
            &format!("Building document context from {} chunks", doc_chunks.len()),
        );

        doc_chunks
            .iter()
            .zip(chunk_sources.iter())
            .enumerate()
            .map(|(index, (chunk, src))| format!("{}, {}, {}", index, src, chunk))
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Truncates edge descriptions based on a maximum token limit and optional prefix filtering.
    ///
    /// # Arguments
    ///
    /// * `edges` - Slice of RelationshipEdge objects
    /// * `chunk_prefix` - Optional string prefix for filtering chunks by source
    /// * `max_tokens` - Maximum number of tokens allowed in the result
    ///
    /// # Returns
    ///
    /// A Result containing either a vector of truncated edge descriptions or a PluginError.
    pub fn truncate_edge_descriptions(
        &self,
        edges: &[RelationshipEdge],
        chunk_prefix: &Option<String>,
        max_tokens: usize,
    ) -> Result<Vec<String>, PluginError> {
        let mut edge_chunks = vec![];

        for edge in edges {
            let sources: Vec<_> = edge
                .doc_source
                .clone()
                .split("<SEP>")
                .map(|s| s.to_string())
                .collect();

            let chunks: Vec<_> = edge
                .description
                .clone()
                .split("<SEP>")
                .map(|s| s.to_string())
                .collect();

            let new_desc = sources
                .iter()
                .zip(chunks.iter())
                .filter_map(|(source, chunk)| {
                    if let Some(ref prefix) = chunk_prefix {
                        if source.starts_with(prefix) {
                            Some(chunk.to_string())
                        } else {
                            None
                        }
                    } else {
                        Some(chunk.to_string())
                    }
                })
                .collect::<Vec<_>>()
                .join("<SEP>");

            if !new_desc.is_empty() && !edge_chunks.contains(&new_desc) {
                edge_chunks.push(new_desc);
            }
        }

        crate::utils::truncate_by_token_size(edge_chunks, max_tokens)
            .map_err(PluginError::Unexpected)
    }

    /// Truncates entity descriptions based on a maximum token limit and optional prefix filtering.
    ///
    /// # Arguments
    ///
    /// * `nodes` - Slice of KgNodeData objects
    /// * `chunk_prefix` - Optional string prefix for filtering chunks by source
    /// * `max_tokens` - Maximum number of tokens allowed in the result
    ///
    /// # Returns
    ///
    /// A Result containing either a vector of truncated entity descriptions or a PluginError.
    pub fn truncate_entity_descriptions(
        &self,
        nodes: &[KgNodeData],
        chunk_prefix: &Option<String>,
        max_tokens: usize,
    ) -> Result<Vec<String>, PluginError> {
        let descriptions = nodes
            .iter()
            .map(|node| {
                if let Some(ref sources) = node.doc_source {
                    let sources: Vec<_> = sources.split("<SEP>").collect();
                    let chunks: Vec<String> = node
                        .description
                        .split("<SEP>")
                        .map(|s| s.to_string())
                        .collect();

                    sources
                        .iter()
                        .zip(chunks)
                        .filter_map(|(source, chunk)| {
                            if let Some(ref prefix) = chunk_prefix {
                                if source.starts_with(prefix) {
                                    Some(chunk)
                                } else {
                                    None
                                }
                            } else {
                                Some(chunk)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("<SEP>")
                } else {
                    node.description.clone()
                }
            })
            .collect();

        crate::utils::truncate_by_token_size(descriptions, max_tokens)
            .map_err(PluginError::Unexpected)
    }

    /// Builds a ContextData struct from entity, edge, and document contexts.
    ///
    /// # Arguments
    ///
    /// * `entity_ctx` - Formatted entity context string
    /// * `edge_ctx` - Formatted edge context string
    /// * `doc_ctx` - Formatted document context string
    ///
    /// # Returns
    ///
    /// A ContextData struct containing the provided contexts.
    pub fn build_context_data(
        &self,
        entity_ctx: String,
        edge_ctx: String,
        doc_ctx: String,
    ) -> ContextData {
        ContextData {
            entity_ctx,
            edge_ctx,
            doc_ctx,
        }
    }
}
