use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::types::{EmbeddingConfig, LLMConfig, SimSearchConfig};
use std::collections::HashMap;

/// Response structure for keyword extraction from an LLM
#[derive(Debug, Deserialize, Serialize, JsonSchema)]

pub struct KeywordsExtractionResponse {
    /// High-level conceptual keywords representing abstract concepts or domains
    #[serde(alias = "high_level_keywords", alias = "highLevelKeywords")]
    pub high_level_keywords: Vec<String>,
    /// Low-level keywords representing specific entities or terms mentioned in the query
    #[serde(alias = "low_level_keywords", alias = "lowLevelKeywords")]
    pub low_level_keywords: Vec<String>,
}

/// Configuration for connecting to and querying a vector database
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]

pub struct VectorDbConfig {
    /// Path or connection string to the vector database
    pub db_path: String,
    /// Name of the table containing entity data
    pub entity_table_name: String,
    /// Name of the table containing relationship data
    pub relationship_table_name: String,
    /// List of entity fields to retrieve from the database
    pub entity_fields: Vec<String>,
    /// List of relationship fields to retrieve from the database
    pub relationship_fields: Vec<String>,
    /// Optional similarity search configuration for vector queries
    pub similarity_search_config: Option<SimSearchConfig>,
}

impl Default for VectorDbConfig {
    fn default() -> Self {
        Self {
            db_path: "".to_string(),
            entity_table_name: "".to_string(),
            relationship_table_name: "".to_string(),
            entity_fields: vec![
                "name".to_string(),
                "content".to_string(),
                "doc_source".to_string(),
            ],
            relationship_fields: vec![
                "id".to_string(),
                "description".to_string(),
                "doc_source".to_string(),
            ],
            similarity_search_config: None,
        }
    }
}

/// Input request for the knowledge graph context retrieval
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]

pub struct ContextRequest {
    /// The user query to extract context for
    pub query: String,
    /// Optional prefix to filter document chunks
    pub chunk_prefix: Option<String>,
    /// Query mode to determine which keywords to use for retrieval
    pub mode: Option<KgMode>,
    /// Maximum number of tokens to include in the context
    pub context_max_tokens: Option<usize>,
    /// Configuration for the vector database
    pub vector_db_config: VectorDbConfig,
    /// Configuration for the LLM used for keyword extraction
    pub llm_config: Option<LLMConfig>,
    /// Configuration for the embedding model used for vector search
    pub embedding_config: Option<EmbeddingConfig>,
}

/// Mode for knowledge graph context retrieval
#[derive(Debug, Default, Copy, Clone, Deserialize, Serialize, JsonSchema)]

pub enum KgMode {
    /// Use low-level keywords only for entity-focused local context
    Local,
    /// Use high-level keywords only for broader conceptual context
    Global,
    /// Use both high and low level keywords for comprehensive context
    #[default]
    Hybrid,
}

/// Response structure containing retrieved knowledge graph context
#[derive(Debug, Deserialize, Serialize, JsonSchema)]

pub struct ContextResponse {
    /// The retrieved knowledge graph context data (serialized JSON)
    pub kg_context: Option<String>,
    /// Information about the keywords used for retrieval
    pub keywords: Option<KeywordInfo>,
}

/// Information about the keywords used for context retrieval
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]

pub struct KeywordInfo {
    /// High-level conceptual keywords used
    pub high_level: Vec<String>,
    /// Low-level specific entity keywords used
    pub low_level: Vec<String>,
    /// The retrieval mode that was used
    pub mode: String,
}

/// Data structure for edge information from the vector database
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VectorDbEdgeData {
    /// Unique identifier for the edge
    pub id: String,
    /// Source node identifier
    pub source: String,
    /// Target node identifier
    pub target: String,
    /// Keywords describing the relationship
    pub keywords: String,
    /// Description of the relationship
    pub description: String,
    /// Strength or weight of the relationship
    pub strength: String,
    /// Reference to source document
    pub doc_source: String,
    /// Additional properties of the edge
    #[serde(flatten)]
    pub properties: HashMap<String, String>,
}

/// Data structure for node information from the vector database
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VectorDbNodeData {
    /// Name of the entity
    pub name: String,
    /// Content or description of the entity
    pub content: String,
    /// Reference to source document
    pub source: String,
    /// Additional properties of the node
    #[serde(flatten)]
    pub properties: HashMap<String, String>,
}

/// Combined context data structure for final output
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContextData {
    /// Context related to entities/nodes
    pub entity_ctx: String,
    /// Context related to relationships/edges
    pub edge_ctx: String,
    /// Context from original document chunks
    pub doc_ctx: String,
}

// Neo4j related data structures

/// Data structure for node information from Neo4j knowledge graph
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KgNodeData {
    /// Type of the entity (e.g., Person, Location, Concept)
    pub entity_type: String,
    /// Description of the entity
    pub description: String,
    /// Unique identifier for the entity
    pub entity_id: String,
    /// Reference to source document, may contain multiple sources separated by <SEP>
    pub doc_source: Option<String>,
    /// Additional properties of the node
    #[serde(flatten)]
    pub properties: HashMap<String, serde_json::Value>,
}

/// Data structure for relationship information from Neo4j knowledge graph
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RelationshipEdge {
    /// Keywords describing the relationship
    pub keywords: String,
    /// Numerical weight/strength of the relationship
    pub weight: f64,
    /// Description of the relationship, may contain multiple descriptions separated by <SEP>
    pub description: String,
    /// Reference to source document, may contain multiple sources separated by <SEP>
    pub doc_source: String,
    /// ID of the target entity
    pub target_entity_id: String,
    /// ID of the source entity
    pub source_entity_id: String,
    /// Unique identifier for the relationship
    pub id: String,
    /// Additional properties of the relationship
    #[serde(flatten)]
    pub properties: HashMap<String, serde_json::Value>,
}

// Neo4j response structures

/// Row structure for node responses from Neo4j
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KgRow {
    /// Array of node data objects
    pub row: Vec<KgNodeData>,
}

/// Response structure for node queries from Neo4j
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeResp {
    /// Status of the request ("success" or error message)
    pub status: String,
    /// Array of result rows
    pub data: Vec<KgRow>,
}

/// Response structure for node degree queries from Neo4j
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeDegreeResponse {
    /// Status of the request ("success" or error message)
    pub status: String,
    /// Data container for degree results
    pub data: NodeDegreeData,
}

/// Data container for node degree results
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeDegreeData {
    /// Array of degree query results
    pub results: Vec<NodeDegreeResult>,
}

/// Structure for node degree query results
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeDegreeResult {
    /// Column names for the result data
    pub columns: Vec<String>,
    /// Array of degree data rows
    pub data: Vec<NodeDegreeRow>,
}

/// Enum for degree row entries which can be either a degree count or entity ID
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DegreeRowEntry {
    /// Numerical degree value (number of connections)
    Degree(i32),
    /// Entity ID string
    EntityId(String),
}

/// Row structure for node degree data
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeDegreeRow {
    /// Array of row entries (typically degree values)
    pub row: Vec<DegreeRowEntry>,
    /// Metadata for the row entries
    pub meta: Vec<Option<serde_json::Value>>,
}

/// Response structure for node edge queries from Neo4j
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeEdgesResponse {
    /// Status of the request ("success" or error message)
    pub status: String,
    /// Data container for edge results
    pub data: NodeEdgesData,
}

/// Data container for node edges results
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeEdgesData {
    /// Array of edge query results
    pub results: Vec<NodeEdgesResult>,
}

/// Structure for node edges query results
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeEdgesResult {
    /// Column names for the result data
    pub columns: Vec<String>,
    /// Array of edge data rows
    pub data: Vec<NodeEdgesRow>,
}

/// Row structure for node edges data
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeEdgesRow {
    /// Tuple of (source node, optional relationship, optional target node)
    pub row: (KgNodeData, Option<RelationshipEdge>, Option<KgNodeData>),
    /// Metadata for the row entries
    pub meta: Vec<Option<NodeMeta>>,
}

/// Response structure for edge queries from Neo4j
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EdgesResponse {
    /// Status of the request ("success" or error message)
    pub status: String,
    /// Array of edge data rows
    pub data: Vec<EdgesRow>,
}

/// Row structure for edge data
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EdgesRow {
    /// Tuple of (source node, target node, relationship)
    pub row: (KgNodeData, KgNodeData, RelationshipEdge),
    /// Metadata for the row entries
    pub meta: Vec<Option<NodeMeta>>,
}

/// Metadata for Neo4j nodes
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeMeta {
    /// Internal Neo4j ID
    pub id: i64,
    /// Element ID string
    #[serde(rename = "elementId")]
    pub element_id: String,
    /// Type of the node
    #[serde(rename = "type")]
    pub node_type: String,
    /// Flag indicating if the node has been deleted
    pub deleted: bool,
}
