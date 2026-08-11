use crate::context_builder::ContextBuilder;
use crate::kg_client::KgClient;
use crate::models::*;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::{
    host::{log, update_status, DbConnection},
    logger::Level,
    open_a_i_like::Client,
};
use crate::vector_db::VectorDbClient;
use serde_json::Value;
use shared::types::EmbeddingConfig;

/// A trait that defines the interface for different context retrieval strategies.
/// Each strategy implements a different approach for extracting relevant information
/// from a knowledge graph to build context for generative AI responses.
pub trait ContextStrategy {
    /// Executes the strategy to build context data based on the provided parameters.
    ///
    /// # Parameters
    /// * `request` - Contains user request details and configuration
    /// * `embedding_client` - Client for generating embeddings
    /// * `db` - Database connection for retrieving information
    /// * `kg_auth` - Authentication information for knowledge graph access
    /// * `high_level_keywords` - Keywords for broader, conceptual matching
    /// * `low_level_keywords` - Keywords for specific, detailed matching
    ///
    /// # Returns
    /// * `Result<ContextData, PluginError>` - Structured context data or an error
    #[allow(clippy::too_many_arguments)]
    fn execute(
        &self,
        request: &ContextRequest,
        embedding_client: &Client,
        embedding_config: &EmbeddingConfig,
        db: &DbConnection,
        kg_auth: &Value,
        high_level_keywords: &[String],
        low_level_keywords: &[String],
    ) -> Result<ContextData, PluginError>;
}

/// Strategy focused on specific entity-level information retrieval.
/// Prioritizes getting detailed information about specific entities
/// using low-level keywords for precision matching.
pub struct LocalStrategy<'a> {
    kg_client: KgClient,
    vector_client: VectorDbClient<'a>,
    context_builder: ContextBuilder,
}

impl<'a> LocalStrategy<'a> {
    /// Creates a new LocalStrategy instance.
    ///
    /// # Parameters
    /// * `db` - Database connection for retrieving vector-based information
    ///
    /// # Returns
    /// * `LocalStrategy` - A new strategy instance
    pub fn new(db: &'a DbConnection) -> LocalStrategy<'a> {
        LocalStrategy {
            kg_client: KgClient::new(),
            vector_client: VectorDbClient::new(db),
            context_builder: ContextBuilder::new(),
        }
    }
}

impl ContextStrategy for LocalStrategy<'_> {
    fn execute(
        &self,
        request: &ContextRequest,
        embedding_client: &Client,
        embedding_config: &EmbeddingConfig,
        _db: &DbConnection,
        kg_auth: &Value,
        _high_level_keywords: &[String],
        low_level_keywords: &[String],
    ) -> Result<ContextData, PluginError> {
        log(
            Level::Info,
            "Executing Local strategy with low-level keywords",
        );

        // 1. Get node data from vector database using low-level keywords
        update_status("Retrieving entities from vector database...");
        let ll_keywords_str = low_level_keywords.join(", ");
        let vector_db_node_data = self.vector_client.get_node_data(
            request,
            embedding_client,
            embedding_config,
            &ll_keywords_str,
        )?;

        // 2. Get corresponding node data from knowledge graph
        update_status("Retrieving entity details from knowledge graph...");
        let mut node_datas = self.kg_client.get_nodes(
            kg_auth,
            vector_db_node_data
                .iter()
                .map(|node| node.name.clone())
                .collect(),
        )?;

        // 3. Get node degrees (connectivity metric)
        let degrees = self.kg_client.get_node_degrees(
            kg_auth,
            node_datas
                .iter()
                .map(|node| node.entity_id.clone())
                .collect(),
        )?;

        // 4. Add degrees to node data
        node_datas
            .iter_mut()
            .zip(degrees)
            .for_each(|(node, degree)| {
                node.properties.insert("degree".to_string(), degree.into());
            });

        // 5. Get document sources from nodes
        let doc_sources = node_datas
            .iter()
            .enumerate()
            .filter_map(|(index, node)| Some((index, node.doc_source.clone()?)))
            .flat_map(|(index, doc_source)| {
                doc_source
                    .split("<SEP>")
                    .map(|s| (index, s.to_string()))
                    .collect::<Vec<(usize, String)>>()
            })
            .collect::<Vec<(usize, String)>>();

        // 6. Get edges for all the nodes
        update_status("Retrieving relationships between entities...");
        let (referenced_chunk_ids, pairs, edges) = self.kg_client.get_nodes_edges(
            kg_auth,
            node_datas
                .iter()
                .map(|node| node.entity_id.clone())
                .collect(),
        )?;

        // 7. Get document chunks
        update_status("Retrieving document chunks...");
        let (doc_chunks, chunk_srcs) =
            self.vector_client
                .get_document_chunks(request, doc_sources, referenced_chunk_ids)?;

        // 8. Sort edges by weight and degree
        update_status("Ranking relationships...");
        let edge_degrees = self.kg_client.get_edges_degrees(kg_auth, pairs)?;
        let mut edge_info: Vec<_> = edges
            .iter()
            .zip(edge_degrees)
            .map(|(e, d)| (e.clone(), d))
            .collect();

        edge_info.sort_by(|(edge_a, degree_a), (edge_b, degree_b)| {
            // Sort by weight (descending) then by degree if weights are equal
            let weight_a = edge_a.weight as u32;
            let weight_b = edge_b.weight as u32;

            match weight_b.cmp(&weight_a) {
                std::cmp::Ordering::Equal => degree_b.cmp(degree_a),
                other => other,
            }
        });

        // 9. Truncate edge descriptions
        let edge_chunks = self.context_builder.truncate_edge_descriptions(
            &edge_info.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>(),
            &request.chunk_prefix,
            request.context_max_tokens.unwrap_or(2000),
        )?;

        edge_info = edge_info[..edge_chunks.len()].to_vec();

        // 10. Truncate entity descriptions
        let entity_chunks = self.context_builder.truncate_entity_descriptions(
            &node_datas,
            &request.chunk_prefix,
            request.context_max_tokens.unwrap_or(2000),
        )?;

        // 11. Build contexts
        update_status("Building context data...");
        let edge_ctx = self
            .context_builder
            .construct_edge_context(edge_chunks, edge_info);
        let chunks_len = entity_chunks.len();
        let entity_ctx = self
            .context_builder
            .construct_entity_context(entity_chunks, &node_datas[..chunks_len]);
        let doc_ctx = self
            .context_builder
            .construct_document_context(doc_chunks, chunk_srcs);

        Ok(self
            .context_builder
            .build_context_data(entity_ctx, edge_ctx, doc_ctx))
    }
}

/// Strategy focused on relationship-level information retrieval.
/// Prioritizes getting information about connections between entities
/// using high-level keywords for conceptual matching.
pub struct GlobalStrategy<'a> {
    kg_client: KgClient,
    vector_client: VectorDbClient<'a>,
    context_builder: ContextBuilder,
}

impl<'a> GlobalStrategy<'a> {
    /// Creates a new GlobalStrategy instance.
    ///
    /// # Parameters
    /// * `db` - Database connection for retrieving vector-based information
    ///
    /// # Returns
    /// * `GlobalStrategy` - A new strategy instance
    pub fn new(db: &'a DbConnection) -> GlobalStrategy<'a> {
        GlobalStrategy {
            kg_client: KgClient::new(),
            vector_client: VectorDbClient::new(db),
            context_builder: ContextBuilder::new(),
        }
    }
}

impl ContextStrategy for GlobalStrategy<'_> {
    fn execute(
        &self,
        request: &ContextRequest,
        embedding_client: &Client,
        embedding_config: &EmbeddingConfig,
        _db: &DbConnection,
        kg_auth: &Value,
        high_level_keywords: &[String],
        _low_level_keywords: &[String],
    ) -> Result<ContextData, PluginError> {
        log(
            Level::Info,
            "Executing Global strategy with high-level keywords",
        );

        // 1. Get edge data from vector database using high-level keywords
        update_status("Retrieving relationships from vector database...");
        let hl_keywords_str = high_level_keywords.join(", ");
        let vector_db_edge_data = self.vector_client.get_edge_data(
            request,
            embedding_client,
            embedding_config,
            &hl_keywords_str,
        )?;

        // 2. Get the source, target and edges from the knowledge graph
        update_status("Retrieving entity and relationship details from knowledge graph...");
        let (src_node_datas, tgt_nodes_data, edges) = self.kg_client.get_edges(
            kg_auth,
            vector_db_edge_data
                .iter()
                .map(|edge| (edge.source.clone(), edge.target.clone()))
                .collect(),
        )?;

        // 3. Get the edge degrees
        let edge_degrees = self.kg_client.get_edges_degrees(
            kg_auth,
            src_node_datas
                .iter()
                .zip(&tgt_nodes_data)
                .map(|(src, tgt)| (src.entity_id.clone(), tgt.entity_id.clone()))
                .collect(),
        )?;

        // 4. Sort the edges by weight and degree
        let mut edge_info: Vec<_> = edges
            .iter()
            .zip(edge_degrees)
            .map(|(e, d)| (e.clone(), d))
            .collect();

        edge_info.sort_by(|(edge_a, degree_a), (edge_b, degree_b)| {
            let weight_a = edge_a.weight as u32;
            let weight_b = edge_b.weight as u32;

            match weight_b.cmp(&weight_a) {
                std::cmp::Ordering::Equal => degree_b.cmp(degree_a),
                other => other,
            }
        });

        // 5. Truncate edge descriptions
        let edge_chunks = self.context_builder.truncate_edge_descriptions(
            &edge_info.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>(),
            &request.chunk_prefix,
            request.context_max_tokens.unwrap_or(2000),
        )?;

        let num_edges = edge_chunks.len();
        edge_info = edge_info[..num_edges].to_vec();
        let edges = edges[..num_edges].to_vec();

        // 6. Get document chunks related to edges
        update_status("Retrieving document chunks for relationships...");
        let indexed_edge_srcs = edge_info
            .iter()
            .enumerate()
            .map(|(i, (edge, _))| (i, edge.doc_source.clone()))
            .collect();

        let (edge_doc_chunks, chunk_srcs) =
            self.vector_client
                .get_document_chunks(request, indexed_edge_srcs, vec![])?;

        // 7. Get entity data for edge endpoints
        update_status("Processing entity data...");
        let mut entity_map = std::collections::HashMap::new();
        for edge in &edges {
            entity_map.insert(edge.source_entity_id.clone(), true);
            entity_map.insert(edge.target_entity_id.clone(), true);
        }

        let entity_ids = entity_map.keys().cloned().collect::<Vec<_>>();
        let mut nodes = self.kg_client.get_nodes(kg_auth, entity_ids)?;

        // 8. Get node degrees
        let degrees = self.kg_client.get_node_degrees(
            kg_auth,
            nodes.iter().map(|node| node.entity_id.clone()).collect(),
        )?;

        // 9. Add degrees to node data
        nodes.iter_mut().zip(degrees).for_each(|(node, degree)| {
            node.properties.insert("degree".to_string(), degree.into());
        });

        // 10. Truncate entity descriptions
        let entity_chunks = self.context_builder.truncate_entity_descriptions(
            &nodes,
            &request.chunk_prefix,
            request.context_max_tokens.unwrap_or(2000),
        )?;

        // 11. Build contexts
        update_status("Building context data...");
        let edge_ctx = self
            .context_builder
            .construct_edge_context(edge_chunks, edge_info);
        let entity_ctx = self
            .context_builder
            .construct_entity_context(entity_chunks.clone(), &nodes[..entity_chunks.len()]);
        let doc_ctx = self
            .context_builder
            .construct_document_context(edge_doc_chunks, chunk_srcs);

        Ok(self
            .context_builder
            .build_context_data(entity_ctx, edge_ctx, doc_ctx))
    }
}

/// Combined strategy that leverages both local and global approaches.
/// This strategy executes both approaches and merges their results,
/// providing comprehensive context that includes both entity details
/// and relationship information.
pub struct HybridStrategy<'a> {
    local_strategy: LocalStrategy<'a>,
    global_strategy: GlobalStrategy<'a>,
    context_builder: ContextBuilder,
}

impl<'a> HybridStrategy<'a> {
    /// Creates a new HybridStrategy instance that combines local and global strategies.
    ///
    /// # Parameters
    /// * `db` - Database connection shared with sub-strategies
    ///
    /// # Returns
    /// * `HybridStrategy` - A new combined strategy instance
    pub fn new(db: &'a DbConnection) -> HybridStrategy<'a> {
        HybridStrategy {
            local_strategy: LocalStrategy::new(db),
            global_strategy: GlobalStrategy::new(db),
            context_builder: ContextBuilder::new(),
        }
    }
}

impl ContextStrategy for HybridStrategy<'_> {
    fn execute(
        &self,
        request: &ContextRequest,
        embedding_client: &Client,
        embedding_config: &EmbeddingConfig,
        db: &DbConnection,
        kg_auth: &Value,
        high_level_keywords: &[String],
        low_level_keywords: &[String],
    ) -> Result<ContextData, PluginError> {
        log(
            Level::Info,
            "Executing Hybrid strategy with both keyword types",
        );

        // Execute both strategies in parallel or sequentially
        update_status("Executing local context strategy...");
        let local_result = self.local_strategy.execute(
            request,
            embedding_client,
            embedding_config,
            db,
            kg_auth,
            high_level_keywords,
            low_level_keywords,
        )?;

        update_status("Executing global context strategy...");
        let global_result = self.global_strategy.execute(
            request,
            embedding_client,
            embedding_config,
            db,
            kg_auth,
            high_level_keywords,
            low_level_keywords,
        )?;

        // Merge results from both strategies
        update_status("Merging context strategies...");
        let entity_ctx = format!(
            "{}\n\n{}",
            local_result.entity_ctx, global_result.entity_ctx
        );
        let edge_ctx = format!("{}\n\n{}", local_result.edge_ctx, global_result.edge_ctx);
        let doc_ctx = format!("{}\n\n{}", local_result.doc_ctx, global_result.doc_ctx);

        Ok(self
            .context_builder
            .build_context_data(entity_ctx, edge_ctx, doc_ctx))
    }
}

/// Factory that creates appropriate context strategy instances based on the specified mode.
/// This follows the Factory Pattern to abstract the creation of strategy objects.
pub struct StrategyFactory {}

impl StrategyFactory {
    /// Creates a strategy instance based on the specified knowledge graph mode.
    ///
    /// # Parameters
    /// * `mode` - The knowledge graph interaction mode (Local, Global, or Hybrid)
    /// * `db` - Database connection to be used by the strategy
    ///
    /// # Returns
    /// * `Box<dyn ContextStrategy + 'a>` - A boxed strategy instance with the database lifetime
    pub fn create_strategy<'a>(
        mode: KgMode,
        db: &'a DbConnection,
    ) -> Box<dyn ContextStrategy + 'a> {
        match mode {
            KgMode::Local => Box::new(LocalStrategy::new(db)),
            KgMode::Global => Box::new(GlobalStrategy::new(db)),
            KgMode::Hybrid => Box::new(HybridStrategy::new(db)),
        }
    }
}
