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
use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};

// host capabilities
use crate::plugin::injector::{
    error::HttpError,
    host::{log, post, update_status},
    http::{HttpRequest, HttpResponse},
    logger::Level,
};

use shared::{inlined_schema_for, TryFromEnvVar};
use std::collections::HashMap;

mod plugin_types;
pub use plugin_types::*;

struct Neo4jKGPlugin;

impl Guest for Neo4jKGPlugin {
    type JsonToJson = Neo4jKG;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "Neo4j KG".to_string(),
            version: "0.1.0".to_string(),
            author: "Neo4j KG".to_string(),
            description: "A plugin for working with Neo4j knowledge graphs".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![("auth".to_string(), "NEO4J_AUTH".to_string())],
            input_schema: serde_json::to_string(&inlined_schema_for!(Neo4jRequest)).unwrap(),
            default_input: serde_json::to_string(&Neo4jRequest {
                auth: Some(Neo4jAuth {
                    uri: "http://localhost:7474".to_string(),
                    username: "neo4j".to_string(),
                    password: "password".to_string(),
                    database: "neo4j".to_string(),
                }),
                operation: Neo4jOperation::GetAllLabels,
            })
            .unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Neo4jResponse)).unwrap(),
        }
    }
}

struct Neo4jKG;

impl Neo4jKG {
    fn run_query(
        &self,
        auth: &Neo4jAuth,
        cypher: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, PluginError> {
        let query_body = serde_json::json!({
            "statements": [{
                "statement": cypher,
                "parameters": params
            }]
        });

        log(Level::Debug, &format!("Running Cypher query: {}", cypher));
        log(Level::Debug, &format!("Query parameters: {:?}", params));

        let res: HttpResponse = post(&HttpRequest {
            url: auth.query_url(),
            headers: vec![
                ("Authorization".to_string(), auth.auth_header()),
                ("Content-Type".to_string(), "application/json".to_string()),
                (
                    "Accept".to_string(),
                    "application/json;charset=UTF-8".to_string(),
                ),
            ],
            body: serde_json::to_string(&query_body)
                .map_err(|e| PluginError::Json(format!("Error serializing query: {}", e)))?
                .as_bytes()
                .to_vec(),
        })?;

        if res.status < 200 || res.status >= 300 {
            return Err(PluginError::Http(HttpError::BadStatus(format!(
                "Neo4j error: HTTP {}, {}",
                res.status,
                String::from_utf8_lossy(&res.body)
            ))));
        }

        let resp: serde_json::Value = serde_json::from_slice(&res.body)
            .map_err(|e| PluginError::Json(format!("Invalid Neo4j response: {}", e)))?;

        // Check for errors in Neo4j response
        if let Some(errors) = resp.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let error_msg = errors[0]
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown Neo4j error");
                return Err(PluginError::Unexpected(format!(
                    "Neo4j query error: {}",
                    error_msg
                )));
            }
        }

        Ok(resp)
    }

    fn run_batch_queries(
        &self,
        auth: &Neo4jAuth,
        statements: Vec<(String, HashMap<String, serde_json::Value>)>,
    ) -> Result<serde_json::Value, PluginError> {
        let mut query_statements = Vec::new();

        for (cypher, params) in statements {
            query_statements.push(serde_json::json!({
                "statement": cypher,
                "parameters": params
            }));
        }

        let query_body = serde_json::json!({
            "statements": query_statements
        });

        log(
            Level::Debug,
            &format!(
                "Running batch queries with {} statements",
                query_statements.len()
            ),
        );

        log(
            Level::Warn,
            &format!(
                "query body: {}",
                serde_json::to_string_pretty(&query_body).unwrap()
            ),
        );

        let res: HttpResponse = post(&HttpRequest {
            url: auth.query_url(),
            headers: vec![
                ("Authorization".to_string(), auth.auth_header()),
                ("Content-Type".to_string(), "application/json".to_string()),
                (
                    "Accept".to_string(),
                    "application/json;charset=UTF-8".to_string(),
                ),
            ],
            body: serde_json::to_string(&query_body)
                .map_err(|e| PluginError::Json(format!("Error serializing batch query: {}", e)))?
                .as_bytes()
                .to_vec(),
        })?;

        if res.status < 200 || res.status >= 300 {
            return Err(PluginError::Http(HttpError::BadStatus(format!(
                "Neo4j batch error: HTTP {}, {}",
                res.status,
                String::from_utf8_lossy(&res.body)
            ))));
        }

        let resp: serde_json::Value = serde_json::from_slice(&res.body)
            .map_err(|e| PluginError::Json(format!("Invalid Neo4j batch response: {}", e)))?;

        // Check for errors in Neo4j response
        if let Some(errors) = resp.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let error_msg = errors[0]
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown Neo4j error");
                return Err(PluginError::Unexpected(format!(
                    "Neo4j batch query error: {}",
                    error_msg
                )));
            }
        }

        Ok(resp)
    }

    fn node_degree_statement(&self, node_id: &str) -> String {
        format!(
            "MATCH (n:base {{entity_id: '{}'}})
        OPTIONAL MATCH (n)-[r]-()
        RETURN COUNT(r) AS degree",
            node_id
        )
    }

    fn node_degree(
        &self,
        auth: &Neo4jAuth,
        node_id: &str,
    ) -> Result<serde_json::Value, PluginError> {
        let cypher = self.node_degree_statement(node_id);
        let params = HashMap::new();
        self.run_query(auth, &cypher, &params)
    }

    fn upsert_node_statement(
        &self,
        node: &Neo4jNode,
    ) -> (String, HashMap<String, serde_json::Value>) {
        let mut properties = node.properties.clone();

        // Ensure required properties are set
        properties.insert(
            "entity_id".to_string(),
            serde_json::Value::String(node.entity_id.clone()),
        );
        properties.insert(
            "entity_type".to_string(),
            serde_json::Value::String(node.entity_type.clone()),
        );

        let mut params = HashMap::new();
        params.insert("properties".to_string(), serde_json::json!(properties));

        params.insert(
            "new_description".to_string(),
            serde_json::Value::String(node.description.clone()),
        );

        if let Some(doc_source) = node.doc_source.clone() {
            params.insert(
                "new_doc_source".to_string(),
                serde_json::Value::String(doc_source),
            );
        }

        (
            if node.doc_source.is_some() {
                format!(
                    "MERGE (n:base {{entity_id: $properties.entity_id}})
                    SET n += $properties
                    SET n.description = CASE
                        WHEN n.description IS NULL THEN $new_description
                        ELSE n.description + '<SEP>' + $new_description
                        END
                    SET n.doc_source = CASE
                        WHEN n.doc_source IS NULL THEN $new_doc_source
                        ELSE n.doc_source + '<SEP>' + $new_doc_source
                        END
                    SET n:`{}`
                    RETURN n",
                    node.entity_type
                )
            } else {
                format!(
                    "MERGE (n:base {{entity_id: $properties.entity_id}})
                    SET n += $properties
                    SET n.description = CASE
                        WHEN n.description IS NULL THEN $new_description
                        ELSE n.description + '<SEP>' + $new_description
                        END
                    SET n:`{}`
                    RETURN n",
                    node.entity_type
                )
            },
            params,
        )
    }

    fn upsert_node(
        &self,
        auth: &Neo4jAuth,
        node: &Neo4jNode,
    ) -> Result<serde_json::Value, PluginError> {
        let (cypher, params) = self.upsert_node_statement(node);

        self.run_query(auth, &cypher, &params)
    }

    fn upsert_edge_statement(
        &self,
        edge: &Neo4jEdge,
    ) -> (String, HashMap<String, serde_json::Value>) {
        let mut properties = HashMap::new();
        for (key, value) in edge.additional_properties.iter() {
            // Skip any JSON objects, only include primitive types
            if !value.is_object() {
                properties.insert(key.clone(), value.clone());
            }
        }

        // Add core properties if provided
        if let Some(weight) = edge.weight {
            properties.insert("weight".to_string(), serde_json::json!(weight));
        }

        // Handle keywords if provided
        if let Some(keywords) = edge.keywords.clone() {
            properties.insert("keywords".to_string(), serde_json::json!(keywords));
        }

        properties.insert(
            "id".to_string(),
            serde_json::json!(format!("{}-{}", edge.source_id, edge.target_id)),
        );

        properties.insert(
            "source_entity_id".to_string(),
            serde_json::json!(edge.source_id),
        );
        properties.insert(
            "target_entity_id".to_string(),
            serde_json::json!(edge.target_id),
        );

        // Extract description for special handling
        let description_to_append = edge.description.clone();
        let src_to_append = edge.doc_source.clone();

        let mut params = HashMap::new();
        params.insert(
            "source_entity_id".to_string(),
            serde_json::json!(edge.source_id),
        );
        params.insert(
            "target_entity_id".to_string(),
            serde_json::json!(edge.target_id),
        );
        params.insert("properties".to_string(), serde_json::json!(properties));

        // Add description as a separate parameter if it exists
        if let Some(desc) = description_to_append.clone() {
            params.insert("new_description".to_string(), serde_json::json!(desc));
        }

        if let Some(src) = src_to_append.clone() {
            params.insert("new_doc_source".to_string(), serde_json::json!(src));
        }

        // Create the Cypher query with conditional description handling
        let description_query_part = if description_to_append.is_some() {
            "
            SET r.description = CASE
                WHEN r.description IS NULL THEN $new_description
                ELSE r.description + '<SEP>' + $new_description
                END
        "
        } else {
            ""
        };

        let doc_source_query_part = if src_to_append.is_some() {
            "
            SET r.doc_source = CASE
                WHEN r.doc_source IS NULL THEN $new_doc_source
                ELSE r.doc_source + '<SEP>' + $new_doc_source
                END"
        } else {
            ""
        };

        let cypher = "
                MATCH (source:base {entity_id: $source_entity_id})
                WITH source
                MATCH (target:base {entity_id: $target_entity_id})
                MERGE (source)-[r:DIRECTED]-(target)
                SET r += $properties
            "
        .to_string()
            + description_query_part
            + doc_source_query_part
            + "
                SET r += $properties
            "
            + "
            RETURN r, source, target";
        (cypher, params)
    }

    fn upsert_edge(
        &self,
        auth: &Neo4jAuth,
        edge: &Neo4jEdge,
    ) -> Result<serde_json::Value, PluginError> {
        let (cypher, params) = self.upsert_edge_statement(edge);
        self.run_query(auth, &cypher, &params)
    }
}

impl GuestJsonToJson for Neo4jKG {
    fn work(&self, input: String) -> Result<String, PluginError> {
        let request: Neo4jRequest = serde_json::from_str(&input).map_err(|e| {
            PluginError::Json(format!(
                "Invalid input: {} -- full input: {}",
                e,
                serde_json::to_value(input).unwrap()
            ))
        })?;

        let auth = match request.auth {
            Some(auth) => auth.clone(),
            None => Neo4jAuth::try_from_env_var("NEO4J_AUTH")
                .map_err(|e| PluginError::EnvVar(format!("Failed to load NEO4J_AUTH: {}", e)))?,
        };

        update_status("Running Neo4j operations...");

        let result = match request.operation {
            Neo4jOperation::UpsertNode { node } => {
                let response = self.upsert_node(&auth, &node)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!("Node {} upserted successfully", node.entity_id)),
                }
            }

            Neo4jOperation::UpsertNodes { nodes } => {
                let mut statements = Vec::new();

                for node in &nodes {
                    statements.push(self.upsert_node_statement(node));
                }

                let response = self.run_batch_queries(&auth, statements)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some(format!("{} nodes upserted successfully", nodes.len())),
                }
            }

            Neo4jOperation::NodeDegree { node_id } => {
                let response = self.node_degree(&auth, &node_id)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!("Node {} degree gotten successfully", node_id)),
                }
            }

            Neo4jOperation::NodeDegrees { node_ids } => {
                let params =
                    HashMap::from([("entity_ids".to_string(), serde_json::json!(node_ids))]);

                let cypher = "
                    UNWIND $entity_ids AS entity_id
                    MATCH (n:base {entity_id: entity_id})
                    OPTIONAL MATCH (n)-[r]-()
                    RETURN COUNT(r) as degree, entity_id
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some(format!(
                        "{} node degrees gotten successfully",
                        node_ids.len()
                    )),
                }
            }

            Neo4jOperation::GetNodeEdges { node_id } => {
                let params = HashMap::from([("entity_id".to_string(), serde_json::json!(node_id))]);

                let cypher = "
                    MATCH (n:base {entity_id: $entity_id})
                    OPTIONAL MATCH (n)-[r]-(connected:base)
                    WHERE connected.entity_id IS NOT NULL
                    RETURN n, properties(r) as edge_properties, connected
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some(format!("{} edges gotten successfully", node_id)),
                }
            }

            Neo4jOperation::GetNodesEdges { node_ids } => {
                let params =
                    HashMap::from([("entity_ids".to_string(), serde_json::json!(node_ids))]);

                let cypher = "
                    UNWIND $entity_ids AS entity_id
                    MATCH (n:base {entity_id: entity_id})
                    OPTIONAL MATCH (n)-[r]-(connected:base)
                    WHERE connected.entity_id IS NOT NULL
                    RETURN n, r, connected
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some(format!("{} node edges gotten successfully", node_ids.len())),
                }
            }

            Neo4jOperation::UpsertEdge { edge } => {
                let response = self.upsert_edge(&auth, &edge)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!(
                        "Edge from {} to {} upserted successfully",
                        edge.source_id, edge.target_id
                    )),
                }
            }

            Neo4jOperation::UpsertEdges { edges } => {
                let mut statements = Vec::new();

                for edge in &edges {
                    statements.push(self.upsert_edge_statement(edge));
                }

                let response = self.run_batch_queries(&auth, statements)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some(format!("{} edges upserted successfully", edges.len())),
                }
            }

            Neo4jOperation::GetNode { node_id } => {
                let mut params = HashMap::new();
                params.insert("entity_id".to_string(), serde_json::json!(node_id));

                let cypher = "MATCH (n:base {entity_id: $entity_id}) RETURN n";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: None,
                }
            }

            Neo4jOperation::GetNodes { node_ids } => {
                let params =
                    HashMap::from([("entity_ids".to_string(), serde_json::json!(node_ids))]);

                let cypher = "
                    UNWIND $entity_ids AS entity_id
                    MATCH (n:base {entity_id: entity_id})
                    RETURN n
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: None,
                }
            }

            Neo4jOperation::GetEdge {
                source_id,
                target_id,
            } => {
                let mut params = HashMap::new();
                params.insert("source_entity_id".to_string(), serde_json::json!(source_id));
                params.insert("target_entity_id".to_string(), serde_json::json!(target_id));

                let cypher = "
                    MATCH (start:base {entity_id: $source_entity_id})-[r]-(end:base {entity_id: $target_entity_id})
                    RETURN start, end, properties(r) as edge_properties
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: None,
                }
            }

            Neo4jOperation::GetEdges { pairs } => {
                if pairs.is_empty() {
                    return Ok(serde_json::to_string(&Neo4jResponse {
                        status: "success".to_string(),
                        data: serde_json::json!({ "results": [{ "data": [] }] }),
                        message: Some("No edge pairs specified".to_string()),
                    })
                    .unwrap());
                }

                // Convert pairs to array of objects to use in UNWIND
                let edge_pairs: Vec<serde_json::Value> = pairs
                    .iter()
                    .map(|(source, target)| {
                        serde_json::json!({
                            "source": source,
                            "target": target
                        })
                    })
                    .collect();

                let mut params = HashMap::new();
                params.insert("pairs".to_string(), serde_json::json!(edge_pairs));

                let cypher = "
                    UNWIND $pairs AS pair
                    MATCH (start:base {entity_id: pair.source})-[r]-(end:base {entity_id: pair.target})
                    RETURN start, end, properties(r) as edge_properties
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: None,
                }
            }

            Neo4jOperation::DeleteNode { node_id } => {
                let mut params = HashMap::new();
                params.insert("entity_id".to_string(), serde_json::json!(node_id));

                let cypher = "
                    MATCH (n:base {entity_id: $entity_id})
                    DETACH DELETE n
                    RETURN count(n) as deleted_count
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!("Node {} deleted", node_id)),
                }
            }

            Neo4jOperation::DeleteNodes { node_ids } => {
                let params =
                    HashMap::from([("entity_ids".to_string(), serde_json::json!(node_ids))]);

                let cypher = "
                    UNWIND $entity_ids AS entity_id
                    MATCH (n:base {entity_id: entity_id})
                    DETACH DELETE n
                    RETURN count(n) as deleted_count
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!("Nodes deleted: {}", node_ids.len())),
                }
            }

            Neo4jOperation::DeleteEdge {
                source_id,
                target_id,
            } => {
                let mut params = HashMap::new();
                params.insert("source_entity_id".to_string(), serde_json::json!(source_id));
                params.insert("target_entity_id".to_string(), serde_json::json!(target_id));

                let cypher = "
                    MATCH (source:base {entity_id: $source_entity_id})-[r]-(target:base {entity_id: $target_entity_id})
                    DELETE r
                    RETURN count(r) as deleted_count
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!("Edge from {} to {} deleted", source_id, target_id)),
                }
            }

            Neo4jOperation::DeleteEdges { pairs } => {
                if pairs.is_empty() {
                    return Ok(serde_json::to_string(&Neo4jResponse {
                        status: "success".to_string(),
                        data: serde_json::json!({ "results": [{ "data": [] }] }),
                        message: Some("No edge pairs specified".to_string()),
                    })
                    .unwrap());
                }

                // Convert pairs to array of objects to use in UNWIND
                let edge_pairs: Vec<serde_json::Value> = pairs
                    .iter()
                    .map(|(source, target)| {
                        serde_json::json!({
                            "source": source,
                            "target": target
                        })
                    })
                    .collect();

                let mut params = HashMap::new();
                params.insert("pairs".to_string(), serde_json::json!(edge_pairs));

                let cypher = "
                    UNWIND $pairs AS pair
                    MATCH (source:base {entity_id: pair.source})-[r]-(target:base {entity_id: pair.target})
                    DELETE r
                    RETURN count(r) as deleted_count
                ";

                let response = self.run_query(&auth, cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: Some(format!("Deleted {} edges", pairs.len())),
                }
            }

            Neo4jOperation::GetKnowledgeGraph { query } => {
                let max_depth = query.max_depth.unwrap_or(3);
                let min_degree = query.min_degree.unwrap_or(0);
                let inclusive = query.inclusive.unwrap_or(false);

                let mut params = HashMap::new();
                params.insert("entity_id".to_string(), serde_json::json!(query.node_label));
                params.insert("max_depth".to_string(), serde_json::json!(max_depth));
                params.insert("min_degree".to_string(), serde_json::json!(min_degree));
                params.insert("inclusive".to_string(), serde_json::json!(inclusive));
                params.insert("max_nodes".to_string(), serde_json::json!(1000)); // MAX_GRAPH_NODES default

                let cypher = if query.node_label == "*" {
                    "
                    MATCH (n)
                    OPTIONAL MATCH (n)-[r]-()
                    WITH n, COALESCE(count(r), 0) AS degree
                    WHERE degree >= $min_degree
                    ORDER BY degree DESC
                    LIMIT $max_nodes
                    WITH collect({node: n}) AS filtered_nodes
                    UNWIND filtered_nodes AS node_info
                    WITH collect(node_info.node) AS kept_nodes, filtered_nodes
                    OPTIONAL MATCH (a)-[r]-(b)
                    WHERE a IN kept_nodes AND b IN kept_nodes
                    RETURN filtered_nodes AS node_info,
                           collect(DISTINCT r) AS relationships
                    "
                } else {
                    "
                    MATCH (start)
                    WHERE
                        CASE
                            WHEN $inclusive THEN start.entity_id CONTAINS $entity_id
                            ELSE start.entity_id = $entity_id
                        END
                    WITH start
                    CALL apoc.path.subgraphAll(start, {
                        relationshipFilter: '',
                        minLevel: 0,
                        maxLevel: $max_depth,
                        bfs: true
                    })
                    YIELD nodes, relationships
                    WITH start, nodes, relationships
                    UNWIND nodes AS node
                    OPTIONAL MATCH (node)-[r]-()
                    WITH node, COALESCE(count(r), 0) AS degree, start, nodes, relationships
                    WHERE node = start OR EXISTS((start)--(node)) OR degree >= $min_degree
                    ORDER BY
                        CASE
                            WHEN node = start THEN 3
                            WHEN EXISTS((start)--(node)) THEN 2
                            ELSE 1
                        END DESC,
                        degree DESC
                    LIMIT $max_nodes
                    WITH collect({node: node}) AS filtered_nodes
                    UNWIND filtered_nodes AS node_info
                    WITH collect(node_info.node) AS kept_nodes, filtered_nodes
                    OPTIONAL MATCH (a)-[r]-(b)
                    WHERE a IN kept_nodes AND b IN kept_nodes
                    RETURN filtered_nodes AS node_info,
                           collect(DISTINCT r) AS relationships
                    "
                };

                // Try with APOC first
                let response = self.run_query(&auth, cypher, &params);

                // If APOC fails, use fallback for non-wildcard queries
                let result = match response {
                    Ok(data) => Ok(data),
                    Err(error) => {
                        if query.node_label != "*" {
                            log(
                                Level::Warn,
                                "APOC plugin error, falling back to basic query",
                            );

                            // Fallback query without APOC
                            let fallback_cypher = "
                                MATCH (n:base {entity_id: $entity_id})
                                OPTIONAL MATCH path = (n)-[*1..$max_depth]-(related)
                                WITH n, related, path
                                WHERE related IS NOT NULL
                                WITH n, related, min(length(path)) as distance
                                OPTIONAL MATCH (related)-[r]-()
                                WITH n, related, distance, count(r) as degree
                                WHERE related = n OR distance = 1 OR degree >= $min_degree
                                WITH collect(n) + collect(related) as nodes
                                UNWIND nodes as node
                                WITH DISTINCT node
                                OPTIONAL MATCH (node)-[r]-(other)
                                WHERE other IN nodes
                                RETURN collect(DISTINCT node) as nodes,
                                       collect(DISTINCT r) as relationships
                                LIMIT $max_nodes
                            ";

                            self.run_query(&auth, fallback_cypher, &params)
                        } else {
                            Err(error)
                        }
                    }
                }?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: result,
                    message: Some(format!(
                        "Knowledge graph for {} retrieved",
                        query.node_label
                    )),
                }
            }

            Neo4jOperation::GetAllLabels => {
                let cypher = "
                    MATCH (n)
                    WHERE n.entity_id IS NOT NULL
                    RETURN DISTINCT n.entity_id AS label
                    ORDER BY label
                ";

                let response = self.run_query(&auth, cypher, &HashMap::new())?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response["results"][0]["data"].clone(),
                    message: None,
                }
            }
            Neo4jOperation::RunCypher { cypher, params } => {
                let response = self.run_query(&auth, &cypher, &params)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some("Custom Cypher query executed".to_string()),
                }
            }

            Neo4jOperation::BatchOperations { operations } => {
                let mut statements = Vec::new();
                let mut messages = Vec::new();

                for op in operations {
                    match op {
                        Neo4jBatchOperation::UpsertNode { node } => {
                            statements.push(self.upsert_node_statement(&node));
                            messages.push(format!("Upserted node {}", node.entity_id));
                        }

                        Neo4jBatchOperation::UpsertEdge { edge } => {
                            statements.push(self.upsert_edge_statement(&edge));
                            messages.push(format!(
                                "Upserted edge from {} to {}",
                                edge.source_id, edge.target_id
                            ));
                        }

                        Neo4jBatchOperation::DeleteNode { node_id } => {
                            let mut params = HashMap::new();
                            params.insert("entity_id".to_string(), serde_json::json!(node_id));

                            let cypher = "
                                MATCH (n:base {entity_id: $entity_id})
                                DETACH DELETE n
                                RETURN count(n) as deleted_count
                            ";

                            statements.push((cypher.to_string(), params));
                            messages.push(format!("Deleted node {}", node_id));
                        }

                        Neo4jBatchOperation::DeleteEdge {
                            source_id,
                            target_id,
                        } => {
                            let mut params = HashMap::new();
                            params.insert(
                                "source_entity_id".to_string(),
                                serde_json::json!(source_id),
                            );
                            params.insert(
                                "target_entity_id".to_string(),
                                serde_json::json!(target_id),
                            );

                            let cypher = "
                                MATCH (source:base {entity_id: $source_entity_id})-[r]-(target:base {entity_id: $target_entity_id})
                                DELETE r
                                RETURN count(r) as deleted_count
                            ";

                            statements.push((cypher.to_string(), params));
                            messages
                                .push(format!("Deleted edge from {} to {}", source_id, target_id));
                        }

                        Neo4jBatchOperation::RunCypher { cypher, params } => {
                            statements.push((cypher, params));
                            messages.push("Executed custom Cypher query".to_string());
                        }
                    }
                }

                let response = self.run_batch_queries(&auth, statements)?;

                Neo4jResponse {
                    status: "success".to_string(),
                    data: response,
                    message: Some(format!(
                        "Executed batch operations: {}",
                        messages.join("; ")
                    )),
                }
            }
        };

        serde_json::to_string(&result)
            .map_err(|e| PluginError::Json(format!("Error serializing response: {}", e)))
    }

    fn new() -> Self {
        Self {}
    }
}

export!(Neo4jKGPlugin);
