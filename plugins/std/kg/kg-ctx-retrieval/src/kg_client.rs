use crate::models::*;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::{
    host::{call_plugin, log},
    logger::Level,
};
use serde_json::Value;

/// A client for interacting with a Knowledge Graph database
///
/// Provides methods to retrieve nodes, edges, and related information from a Neo4j
/// knowledge graph through a plugin interface.
pub struct KgClient {}

impl KgClient {
    /// Creates a new KgClient instance
    ///
    /// # Returns
    /// A new instance of the KgClient
    pub fn new() -> Self {
        Self {}
    }

    /// Retrieves node data for specified node IDs from the knowledge graph
    ///
    /// # Arguments
    /// * `kg_auth` - Authentication information for the knowledge graph
    /// * `node_ids` - Vector of node identifiers to retrieve
    ///
    /// # Returns
    /// * `Ok(Vec<KgNodeData>)` - Vector of node data on success
    /// * `Err(PluginError)` - Error if the operation fails
    pub fn get_nodes(
        &self,
        kg_auth: &Value,
        node_ids: Vec<String>,
    ) -> Result<Vec<KgNodeData>, PluginError> {
        log(
            Level::Info,
            &format!("Getting nodes for {} IDs", node_ids.len()),
        );

        let resp = call_plugin(
            "neo4j-kg",
            &serde_json::json!({
                "auth": kg_auth,
                "operation": {
                    "operation": "get_nodes",
                    "node_ids": node_ids
                }
            })
            .to_string(),
        )?;

        let response_data: NodeResp =
            serde_json::from_str(&resp).map_err(|e| PluginError::Json(e.to_string()))?;

        if response_data.status != "success" {
            return Err(PluginError::KgDb("Get entities from KG failed".to_string()));
        }

        Ok(response_data
            .data
            .into_iter()
            .map(|mut row| row.row.pop().unwrap())
            .collect())
    }

    /// Retrieves the degree (number of connections) for specified nodes
    ///
    /// # Arguments
    /// * `kg_auth` - Authentication information for the knowledge graph
    /// * `node_ids` - Vector of node identifiers to retrieve degrees for
    ///
    /// # Returns
    /// * `Ok(Vec<i32>)` - Vector of degree values for each node
    /// * `Err(PluginError)` - Error if the operation fails
    pub fn get_node_degrees(
        &self,
        kg_auth: &Value,
        node_ids: Vec<String>,
    ) -> Result<Vec<i32>, PluginError> {
        if node_ids.is_empty() {
            return Ok(vec![]);
        }

        log(
            Level::Info,
            &format!("Getting node degrees for {} IDs", node_ids.len()),
        );

        let node_degrees = call_plugin(
            "neo4j-kg",
            &serde_json::json!({
                "auth": kg_auth,
                "operation": {
                    "operation": "node_degrees",
                    "node_ids": node_ids
                }
            })
            .to_string(),
        )?;

        let node_degrees: NodeDegreeResponse = serde_json::from_str(&node_degrees)
            .map_err(|e| PluginError::Json(format!("Failed to parse node degrees: {}", e)))?;

        // Extract degree values from response
        let degrees: Vec<i32> = node_degrees
            .data
            .results
            .iter()
            .flat_map(|row| {
                row.data
                    .iter()
                    .filter_map(|row| {
                        if let DegreeRowEntry::Degree(d) = row.row[0] {
                            Some(d)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        Ok(degrees)
    }

    /// Retrieves edges connected to specified nodes
    ///
    /// # Arguments
    /// * `kg_auth` - Authentication information for the knowledge graph
    /// * `ids` - Vector of node identifiers to retrieve edges for
    ///
    /// # Returns
    /// * `Ok((Vec<String>, Vec<(String, String)>, Vec<RelationshipEdge>))` - Tuple containing:
    ///   - Referenced chunk IDs
    ///   - Node pairs (source ID, target ID)
    ///   - Relationship edges between nodes
    /// * `Err(PluginError)` - Error if the operation fails
    #[allow(clippy::type_complexity)]
    pub fn get_nodes_edges(
        &self,
        kg_auth: &Value,
        ids: Vec<String>,
    ) -> Result<(Vec<String>, Vec<(String, String)>, Vec<RelationshipEdge>), PluginError> {
        if ids.is_empty() {
            return Ok((vec![], vec![], vec![]));
        }

        log(
            Level::Info,
            &format!("Getting edges for {} nodes", ids.len()),
        );

        let node_edges = call_plugin(
            "neo4j-kg",
            &serde_json::json!({
                "auth": kg_auth,
                "operation": {
                    "operation": "get_nodes_edges",
                    "node_ids": ids,
                }
            })
            .to_string(),
        )?;

        let node_edge_resp: NodeEdgesResponse =
            serde_json::from_str(&node_edges).map_err(|e| PluginError::Json(e.to_string()))?;

        // Extract all target nodes from NodeEdgesResponse
        let mut referenced_chunk_ids = Vec::new();
        let mut pairs = Vec::new();
        let mut edges = Vec::new();

        let mut seen_nodes = Vec::new();
        if let Some(results) = node_edge_resp.data.results.first() {
            for row in &results.data {
                // Each row has (src_node, edge, target_node)
                if let Some(edge) = &row.row.1 {
                    edges.push(edge.clone());
                }

                if let Some(target_node) = &row.row.2 {
                    if seen_nodes.contains(&target_node.entity_id) {
                        continue;
                    }
                    seen_nodes.push(target_node.entity_id.clone());
                    if let Some(ref src) = target_node.doc_source {
                        referenced_chunk_ids.push(src.clone());
                    }
                    pairs.push((row.row.0.entity_id.clone(), target_node.entity_id.clone()));
                }
            }
        }

        Ok((referenced_chunk_ids, pairs, edges))
    }

    /// Retrieves edge data for specified node pairs
    ///
    /// # Arguments
    /// * `kg_auth` - Authentication information for the knowledge graph
    /// * `pairs` - Vector of node pairs (source ID, target ID) to retrieve edges for
    ///
    /// # Returns
    /// * `Ok((Vec<KgNodeData>, Vec<KgNodeData>, Vec<RelationshipEdge>))` - Tuple containing:
    ///   - Source node data
    ///   - Target node data
    ///   - Relationship edges between nodes
    /// * `Err(PluginError)` - Error if the operation fails
    #[allow(clippy::type_complexity)]
    pub fn get_edges(
        &self,
        kg_auth: &Value,
        pairs: Vec<(String, String)>,
    ) -> Result<(Vec<KgNodeData>, Vec<KgNodeData>, Vec<RelationshipEdge>), PluginError> {
        if pairs.is_empty() {
            return Ok((vec![], vec![], vec![]));
        }

        log(
            Level::Info,
            &format!("Getting edges for {} pairs", pairs.len()),
        );

        let resp = call_plugin(
            "neo4j-kg",
            &serde_json::json!({
                "auth": kg_auth,
                "operation": {
                    "operation": "get_edges",
                    "pairs": pairs
                }
            })
            .to_string(),
        )?;

        let response_data: EdgesResponse =
            serde_json::from_str(&resp).map_err(|e| PluginError::Json(e.to_string()))?;

        if response_data.status != "success" {
            return Err(PluginError::KgDb("Get entities from KG failed".to_string()));
        }

        let (src_node_datas, tgt_node_datas, edge_datas): (
            Vec<KgNodeData>,
            Vec<KgNodeData>,
            Vec<RelationshipEdge>,
        ) = response_data.data.into_iter().map(|row| row.row).collect();

        Ok((src_node_datas, tgt_node_datas, edge_datas))
    }

    /// Calculates the sum of degrees for each node pair
    ///
    /// # Arguments
    /// * `kg_auth` - Authentication information for the knowledge graph
    /// * `pairs` - Vector of node pairs (source ID, target ID) to calculate degrees for
    ///
    /// # Returns
    /// * `Ok(Vec<i32>)` - Vector containing the sum of source and target node degrees for each pair
    /// * `Err(PluginError)` - Error if the operation fails
    pub fn get_edges_degrees(
        &self,
        kg_auth: &Value,
        pairs: Vec<(String, String)>,
    ) -> Result<Vec<i32>, PluginError> {
        if pairs.is_empty() {
            return Ok(vec![]);
        }

        let src_hop_node_degrees = self.get_node_degrees(
            kg_auth,
            pairs
                .iter()
                .map(|node| node.0.clone())
                .collect::<Vec<String>>(),
        )?;

        let tgt_hop_node_degrees = self.get_node_degrees(
            kg_auth,
            pairs
                .iter()
                .map(|node| node.1.clone())
                .collect::<Vec<String>>(),
        )?;

        Ok(src_hop_node_degrees
            .iter()
            .zip(tgt_hop_node_degrees)
            .map(|(src_row, tgt_row)| src_row + tgt_row)
            .collect())
    }
}
