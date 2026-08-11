use base64::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Authentication information for connecting to a Neo4j database.
///
/// Contains all the necessary credentials and connection details to establish
/// a connection to a Neo4j instance.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Neo4jAuth {
    /// The URI of the Neo4j server, including protocol and port
    pub uri: String,
    /// The username for authentication
    pub username: String,
    /// The password for authentication
    pub password: String,
    /// The specific database name to connect to
    pub database: String,
}

impl Neo4jAuth {
    /// Generates a Basic authentication header for HTTP requests.
    ///
    /// Returns a string in the format "Basic <base64-encoded-credentials>".
    pub fn auth_header(&self) -> String {
        format!(
            "Basic {}",
            BASE64_STANDARD.encode(format!("{}:{}", self.username, self.password))
        )
    }

    /// Constructs the full URL for Neo4j transaction endpoint.
    ///
    /// Returns the complete URL to use for query execution.
    pub fn query_url(&self) -> String {
        format!("{}/db/{}/tx/commit", self.uri, self.database)
    }
}

/// Represents a node in the Neo4j graph database.
///
/// Contains the core properties of a node along with any additional custom properties.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Neo4jNode {
    /// Unique identifier for the node
    pub entity_id: String,
    /// Type/category of the entity this node represents
    pub entity_type: String,
    /// Human-readable description of the node
    pub description: String,
    /// Optional source document reference
    pub doc_source: Option<String>,
    /// Additional custom properties associated with this node
    #[serde(flatten)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// Represents an edge (relationship) in the Neo4j graph database.
///
/// Defines a connection between two nodes with associated metadata.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Neo4jEdge {
    /// ID of the source node where the edge starts
    pub source_id: String,
    /// ID of the target node where the edge ends
    pub target_id: String,
    /// Optional weight/strength of the relationship
    pub weight: Option<f64>,
    /// Optional human-readable description of the relationship
    pub description: Option<String>,
    /// Optional keywords associated with this relationship
    pub keywords: Option<String>,
    /// Optional source document reference
    pub doc_source: Option<String>,
    /// Additional custom properties associated with this edge
    #[serde(flatten)]
    pub additional_properties: HashMap<String, serde_json::Value>,
}

/// Response structure for Neo4j operations.
///
/// Contains the result of a Neo4j query operation.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Neo4jResponse {
    /// Status of the operation (success, error, etc.)
    pub status: String,
    /// The data returned by the operation
    pub data: serde_json::Value,
    /// Optional message providing additional information
    pub message: Option<String>,
}

/// Parameters for querying a knowledge graph.
///
/// Used to specify parameters for graph traversal and retrieval.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GraphQuery {
    /// The label of the node to start traversal from
    pub node_label: String,
    /// Maximum depth for traversal (number of hops)
    pub max_depth: Option<i32>,
    /// Minimum degree (number of connections) for included nodes
    pub min_degree: Option<i32>,
    /// Whether to include nodes that don't meet the criteria but are connected to ones that do
    pub inclusive: Option<bool>,
}

/// Enum representing all possible operations that can be performed on Neo4j.
///
/// Each variant corresponds to a specific database operation.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation")]
pub enum Neo4jOperation {
    /// Create or update a single node
    #[serde(rename = "upsert_node")]
    UpsertNode { node: Neo4jNode },

    /// Get the number of connections for a specific node
    #[serde(rename = "node_degree")]
    NodeDegree { node_id: String },

    /// Get the number of connections for multiple nodes
    #[serde(rename = "node_degrees")]
    NodeDegrees { node_ids: Vec<String> },

    /// Create or update multiple nodes in a single operation
    #[serde(rename = "upsert_nodes")]
    UpsertNodes { nodes: Vec<Neo4jNode> },

    /// Create or update a single edge between nodes
    #[serde(rename = "upsert_edge")]
    UpsertEdge { edge: Neo4jEdge },

    /// Create or update multiple edges in a single operation
    #[serde(rename = "upsert_edges")]
    UpsertEdges { edges: Vec<Neo4jEdge> },

    /// Retrieve a single node by its ID
    #[serde(rename = "get_node")]
    GetNode { node_id: String },

    /// Retrieve multiple nodes by their IDs
    #[serde(rename = "get_nodes")]
    GetNodes { node_ids: Vec<String> },

    /// Get all edges connected to a specific node
    #[serde(rename = "get_node_edges")]
    GetNodeEdges { node_id: String },

    /// Get all edges connected to multiple nodes
    #[serde(rename = "get_nodes_edges")]
    GetNodesEdges { node_ids: Vec<String> },

    /// Get a specific edge between two nodes
    #[serde(rename = "get_edge")]
    GetEdge {
        source_id: String,
        target_id: String,
    },

    /// Get multiple specific edges between pairs of nodes
    #[serde(rename = "get_edges")]
    GetEdges { pairs: Vec<(String, String)> },

    /// Delete a node by its ID
    #[serde(rename = "delete_node")]
    DeleteNode { node_id: String },

    /// Delete multiple nodes by their IDs
    #[serde(rename = "delete_nodes")]
    DeleteNodes { node_ids: Vec<String> },

    /// Delete a specific edge between two nodes
    #[serde(rename = "delete_edge")]
    DeleteEdge {
        source_id: String,
        target_id: String,
    },

    /// Delete multiple edges between pairs of nodes
    #[serde(rename = "delete_edges")]
    DeleteEdges { pairs: Vec<(String, String)> },

    /// Retrieve a knowledge graph based on query parameters
    #[serde(rename = "get_knowledge_graph")]
    GetKnowledgeGraph { query: GraphQuery },

    /// Get all node labels in the database
    #[serde(rename = "get_all_labels")]
    GetAllLabels,

    /// Execute a custom Cypher query with parameters
    #[serde(rename = "run_cypher")]
    RunCypher {
        cypher: String,
        params: HashMap<String, serde_json::Value>,
    },

    /// Execute multiple operations in a single batch
    #[serde(rename = "batch_operations")]
    BatchOperations {
        operations: Vec<Neo4jBatchOperation>,
    },
}

/// Operations that can be included in a batch request.
///
/// A subset of operations from Neo4jOperation that can be batched together.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Neo4jBatchOperation {
    /// Create or update a single node in a batch
    #[serde(rename = "upsert_node")]
    UpsertNode { node: Neo4jNode },

    /// Create or update a single edge in a batch
    #[serde(rename = "upsert_edge")]
    UpsertEdge { edge: Neo4jEdge },

    /// Delete a node in a batch
    #[serde(rename = "delete_node")]
    DeleteNode { node_id: String },

    /// Delete an edge in a batch
    #[serde(rename = "delete_edge")]
    DeleteEdge {
        source_id: String,
        target_id: String,
    },

    /// Execute a custom Cypher query in a batch
    #[serde(rename = "run_cypher")]
    RunCypher {
        cypher: String,
        params: HashMap<String, serde_json::Value>,
    },
}

/// Request structure for Neo4j operations.
///
/// Contains authentication details and the operation to perform.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Neo4jRequest {
    /// Authentication credentials for the Neo4j connection
    pub auth: Option<Neo4jAuth>,
    /// The operation to perform on the database
    pub operation: Neo4jOperation,
}
