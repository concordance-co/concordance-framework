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
use crate::plugin::injector::{
    host::{connect_db, new_client, update_status},
    open_a_i_like::Client,
    vector_db::{DbConnection, SimilaritySearchConfig},
};

use shared::types::{EmbeddingConfig, SimSearchConfig};

use arrow_schema::{DataType, Field, Schema as ArrowSchema};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::{inlined_schema_for, with_examples_inlined_schema_for, TryFromEnvVar};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Request {
    /// The action to perform
    pub action: Action,
    /// The path to the database file
    pub db_path: String,
    /// The name of the table to create or add data to
    pub table_name: String,
    /// The configuration for the embedding model. This should always be null
    /// when using LLM function calling
    pub embedding_config: Option<EmbeddingConfig>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            action: Action::default(),
            db_path: "".to_string(),
            table_name: "".to_string(),
            embedding_config: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", untagged)]
pub enum Action {
    AddRows {
        /// The rows to add to the table
        rows: Vec<serde_json::Value>,
        /// Which entry in a row to embed
        embedded_row_key: String,
        /// The entry in a row to use as the primary key for upserts
        upsert_on: Option<String>,
        /// Create table if it doesn't exist
        create_if_not_exists: Option<bool>,
    },
    CreateTable {
        /// The schema of the table to create
        schema_json_str: String,
    },
    AddColumns {
        /// The columns to add to the table
        columns: Vec<Vec<serde_json::Value>>,
        /// Which column to embed
        embedded_column_index: usize,
        /// The column name to use as the primary key for upserts
        upsert_on: Option<String>,
        /// Column names to use if creating a new table
        column_names: Option<Vec<String>>,
        /// Create table if it doesn't exist
        create_if_not_exists: Option<bool>,
    },
    Search {
        /// A vector of tuples containing the string to search and the corresponding search configuration
        strs: Vec<(String, SimSearchConfig)>,
    },
    GetRow {
        /// The column name to use as the primary key for retrieval
        id_column: String,
        /// The value to use as the primary key for retrieval
        id_value: String,
        /// The column names to return
        fields_returned: Vec<String>,
    },
}

impl Default for Action {
    fn default() -> Self {
        Action::Search { strs: Vec::new() }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Response {
    success: bool,
    data: serde_json::Value,
}

pub struct DocVecStorePlugin;

impl Guest for DocVecStorePlugin {
    type JsonToJson = DocVecStore;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Document Vector Store".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description:
                "An interface for creating and managing document vector stores. Create tables, add rows or columns with embedded text, and perform semantic search operations using LLM embeddings.".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![("embedding_config".to_string(), "EMBEDDING_CONFIG".to_string())],
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(Request, Request::default())).unwrap(),
            default_input: serde_json::to_string(&Request::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Response)).unwrap(),
        }
    }
}

pub struct DocVecStore;

impl GuestJsonToJson for DocVecStore {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Parse request
        let request: Request = serde_json::from_str(&input)
            .map_err(|e| PluginError::Json(format!("Failed to parse request: {}", e)))?;

        let embedding_config = match request.embedding_config {
            Some(ref config) => config.clone(),
            None => EmbeddingConfig::try_from_env_var("EMBEDDING_CONFIG").map_err(|e| {
                PluginError::EnvVar(format!("Failed to load EMBEDDING_CONFIG: {}", e))
            })?,
        };

        // Create clients
        let embedding_client = new_client(&embedding_config.base_url, &embedding_config.api_key)?;
        let db = connect_db(&request.db_path)?;

        // Process the requested action
        let result = match request.action {
            Action::CreateTable { schema_json_str } => {
                update_status("Creating vector database table...");
                let success = db.create_table(&request.table_name, &schema_json_str)?;
                serde_json::json!({ "created": success })
            }

            Action::AddRows {
                rows,
                embedded_row_key,
                create_if_not_exists,
                upsert_on,
            } => {
                update_status("Adding rows to vector database...");

                // Convert rows to columns
                let (columns, column_names) = rows_to_columns(&rows)?;

                // Create table if needed
                ensure_table_exists(
                    &db,
                    &request.table_name,
                    create_if_not_exists,
                    Some((&columns, &column_names)),
                    &embedding_client,
                    &embedding_config.model_name,
                )?;

                // Get schema and prepare data
                let schema = db.get_table_schema_json_str(&request.table_name)?;
                let schema_parsed: ArrowSchema = serde_json::from_str(&schema)
                    .map_err(|e| PluginError::VectorDb(format!("Failed to parse schema: {}", e)))?;

                let field_names: Vec<String> = schema_parsed
                    .fields
                    .iter()
                    .map(|f| f.name().clone())
                    .collect();

                // Find embedding indices
                let to_embed_index = field_names
                    .iter()
                    .position(|name| name == &embedded_row_key)
                    .ok_or_else(|| {
                        PluginError::VectorDb(format!(
                            "Embedded column '{}' not found in schema",
                            embedded_row_key
                        ))
                    })?;

                let embedding_index = schema_parsed
                    .fields
                    .iter()
                    .position(|field| matches!(field.data_type(), DataType::FixedSizeList(..)))
                    .ok_or_else(|| {
                        PluginError::VectorDb("Embedding column not found in schema".to_string())
                    })?;

                // Prepare columns for adding
                let mut to_add: Vec<Vec<serde_json::Value>> = vec![Vec::new(); field_names.len()];
                to_add.remove(embedding_index); // Remove embedding column as it will be generated

                // Fill columns with data from rows
                for row in &rows {
                    if let serde_json::Value::Object(map) = &row {
                        for (i, field_name) in field_names.iter().enumerate() {
                            if i != embedding_index {
                                let col_idx = if i > embedding_index { i - 1 } else { i };
                                to_add[col_idx].push(
                                    map.get(field_name)
                                        .cloned()
                                        .unwrap_or(serde_json::Value::Null),
                                );
                            }
                        }
                    }
                }

                // Convert to JSON strings
                let to_add: Vec<String> = to_add
                    .iter()
                    .map(|column| serde_json::to_string(column).unwrap())
                    .collect();

                // Add to database
                let success = db.add(
                    &request.table_name,
                    embedding_client,
                    &embedding_config.model_name,
                    &to_add,
                    to_embed_index as u32,
                    upsert_on.as_deref(),
                )?;

                serde_json::json!({ "added": success, "count": rows.len() })
            }

            Action::AddColumns {
                columns,
                embedded_column_index,
                column_names,
                create_if_not_exists,
                upsert_on,
            } => {
                update_status("Adding columns to vector database...");

                // Validate embedded column index
                if embedded_column_index >= columns.len() {
                    return Err(PluginError::VectorDb(format!(
                        "Embedded column index {} is out of bounds for {} columns",
                        embedded_column_index,
                        columns.len()
                    )));
                }

                // Create table if needed
                ensure_table_exists(
                    &db,
                    &request.table_name,
                    create_if_not_exists,
                    column_names
                        .as_ref()
                        .map(|names| (&columns[..], &names[..])),
                    &embedding_client,
                    &embedding_config.model_name,
                )?;

                // Convert columns to JSON strings
                let columns_json: Vec<String> = columns
                    .iter()
                    .map(|column| serde_json::to_string(column).unwrap())
                    .collect();

                // Add columns to database
                let success = db.add(
                    &request.table_name,
                    embedding_client,
                    &embedding_config.model_name,
                    &columns_json,
                    embedded_column_index as u32,
                    upsert_on.as_deref(),
                )?;

                serde_json::json!({ "added": success, "count": columns.len() })
            }

            Action::Search { strs } => {
                update_status("Performing similarity search on vector database...");

                if strs.is_empty() {
                    return Ok(serde_json::to_string(&Response {
                        success: true,
                        data: serde_json::json!([]),
                    })
                    .unwrap());
                }

                // Perform searches
                let mut all_results = Vec::new();
                for (query_str, config) in strs {
                    let search_config = SimilaritySearchConfig {
                        limit: config.limit,
                        threshold: config.threshold,
                        fields_returned: config.fields_returns,
                        where_clause: config.where_clause,
                        include_embeddings: config.include_embeddings,
                    };

                    let search_results = db.similarity_search(
                        &search_config,
                        &embedding_client,
                        &embedding_config.model_name,
                        &request.table_name,
                        &query_str,
                    )?;

                    all_results.push(
                        serde_json::to_value(&search_results)
                            .map_err(|e| PluginError::Json(e.to_string()))?,
                    );
                }

                serde_json::json!(all_results)
            }

            Action::GetRow {
                id_column,
                id_value,
                fields_returned,
            } => {
                update_status("Performing row lookup on vector database...");

                let row =
                    db.get_row_by_id(&request.table_name, &id_column, &id_value, &fields_returned)?;

                serde_json::json!({ "row": row })
            }
        };

        // Return formatted response
        Ok(serde_json::to_string(&Response {
            success: true,
            data: result,
        })
        .unwrap())
    }

    fn new() -> Self {
        Self {}
    }
}

// Helper function to ensure a table exists
fn ensure_table_exists(
    db: &DbConnection,
    table_name: &str,
    create_if_not_exists: Option<bool>,
    column_data: Option<(&[Vec<serde_json::Value>], &[String])>,
    embedding_client: &Client,
    model_name: &str,
) -> Result<bool, PluginError> {
    let table_names = db.get_table_names()?;

    if !table_names.contains(&table_name.to_string()) {
        if matches!(create_if_not_exists, Some(true)) {
            if let Some((columns, column_names)) = column_data {
                let embeddings_dimensions =
                    embedding_client.get_embeddings_dimensions(model_name)?;
                let schema = infer_schema(columns, column_names, embeddings_dimensions as i32)?;
                db.create_table(table_name, &serde_json::to_string(&schema).unwrap())?;
                return Ok(true);
            } else {
                return Err(PluginError::VectorDb(
                    "Cannot create table: column data not provided".to_string(),
                ));
            }
        } else {
            return Err(PluginError::VectorDb(format!(
                "Table '{}' does not exist and create_if_not_exists is not set to true",
                table_name
            )));
        }
    }

    Ok(false)
}

// Convert rows to columns
fn rows_to_columns(
    rows: &[serde_json::Value],
) -> Result<(Vec<Vec<serde_json::Value>>, Vec<String>), PluginError> {
    if rows.is_empty() {
        return Err(PluginError::VectorDb("No rows provided".to_string()));
    }

    // Get unique keys from all rows
    let mut all_keys = HashSet::new();
    for row in rows {
        if let serde_json::Value::Object(map) = row {
            for key in map.keys() {
                all_keys.insert(key.clone());
            }
        } else {
            return Err(PluginError::VectorDb(
                "Each row must be an object".to_string(),
            ));
        }
    }

    let keys: Vec<String> = all_keys.into_iter().collect();
    let mut columns = vec![Vec::with_capacity(rows.len()); keys.len()];

    // Fill columns with values from rows
    for row in rows {
        if let serde_json::Value::Object(map) = row {
            for (i, key) in keys.iter().enumerate() {
                columns[i].push(map.get(key).cloned().unwrap_or(serde_json::Value::Null));
            }
        }
    }

    Ok((columns, keys))
}

// Simplified schema inference
fn infer_schema(
    columns: &[Vec<serde_json::Value>],
    column_names: &[String],
    embedding_dims: i32,
) -> Result<ArrowSchema, PluginError> {
    if columns.len() != column_names.len() {
        return Err(PluginError::VectorDb(format!(
            "Column count ({}) does not match column names count ({})",
            columns.len(),
            column_names.len()
        )));
    }

    let mut fields = Vec::new();

    for (i, column) in columns.iter().enumerate() {
        let name = &column_names[i];

        // Determine data type from values
        let data_type = if column.is_empty() {
            DataType::Utf8 // Default to string if no data
        } else {
            infer_column_type(column)
        };

        let nullable = column.iter().any(|v| v.is_null());
        fields.push(Field::new(name, data_type, nullable));
    }

    // Add embedding field
    fields.push(Field::new(
        "embeddings",
        DataType::FixedSizeList(
            Arc::new(Field::new("item", DataType::Float32, false)),
            embedding_dims,
        ),
        false,
    ));

    Ok(ArrowSchema::new(fields))
}

// Helper for schema inference
fn infer_column_type(column: &[serde_json::Value]) -> DataType {
    let mut is_number = true;
    let mut is_float = false;
    let mut is_negative = false;
    let mut is_bool = true;
    let mut is_array = true;
    let mut array_ty = None;

    for value in column {
        match value {
            serde_json::Value::Number(n) => {
                is_bool = false;
                is_array = false;
                if n.is_f64() {
                    is_float = true;
                }
                if let Some(i) = n.as_i64() {
                    if i < 0 {
                        is_negative = true;
                    }
                }
            }
            serde_json::Value::Bool(_) => {
                is_number = false;
                is_array = false;
            }
            serde_json::Value::Array(inner) => {
                array_ty = Some(infer_column_type(&[inner[0].clone()]));
                is_number = false;
                is_bool = false;
            }
            _ => {
                is_number = false;
                is_bool = false;
                is_array = false;
            }
        }
    }

    if is_array {
        DataType::List(Arc::new(Field::new(
            "item",
            array_ty.unwrap_or(DataType::Utf8),
            true,
        )))
    } else if is_bool {
        DataType::Boolean
    } else if is_number {
        if is_float {
            DataType::Float64
        } else if is_negative {
            DataType::Int64
        } else {
            DataType::UInt64
        }
    } else {
        DataType::Utf8
    }
}

export!(DocVecStorePlugin);
