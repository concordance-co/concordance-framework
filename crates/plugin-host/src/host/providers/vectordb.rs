//! This module provides vector database functionality.
//!
//! The functionality is conditionally compiled based on the "vectordb" feature flag.
//! When enabled, it provides full LanceDB integration for vector search and storage.
//! When disabled, it provides a minimal stub implementation.

#[cfg(feature = "vectordb")]
pub use vdb::*;

#[cfg(not(feature = "vectordb"))]
pub use nonvdb::*;

#[cfg(feature = "vectordb")]
mod vdb {
    use crate::host::providers::llm::{embeddings_create, Client};
    use crate::injector::error::PluginError;
    use crate::injector::open_a_i_like::EmbeddingInput;
    use crate::injector::vector_db::{SimilarityResponse, SimilaritySearchConfig};
    use arrow_array::types::Float32Type;
    use arrow_array::*;
    use arrow_schema::{DataType, Schema};
    use futures_util::stream::TryStreamExt;
    // use lance_arrow::FixedSizeListArrayExt;
    use lancedb::query::{ExecutableQuery, QueryBase, Select};
    use lancedb::{connect, Connection};
    use std::sync::Arc;
    use wasmtime::component::{Resource, ResourceTable};

    /// Database connection for vector database operations.
    ///
    /// Provides methods for creating and managing tables, adding data with embeddings,
    /// and performing similarity searches.
    #[derive(Clone)]
    pub struct DbConn {
        /// Path to the database file or directory
        pub path: String,
        /// Active database connection
        pub conn: Connection,
    }

    impl DbConn {
        /// Creates a new database connection at the specified path.
        ///
        /// # Arguments
        /// * `path` - Path to the database file or directory
        ///
        /// # Returns
        /// A new `DbConn` instance with an active connection
        pub async fn new(path: String) -> Self {
            let conn = connect(&path).execute().await.unwrap();

            Self { path, conn }
        }

        /// Retrieves a table's schema as a JSON string.
        ///
        /// # Arguments
        /// * `table_name` - Name of the table to get the schema for
        ///
        /// # Returns
        /// JSON representation of the table schema or an error
        pub async fn get_table_schema_json_str(
            &self,
            table_name: &str,
        ) -> Result<String, PluginError> {
            let table = self
                .conn
                .open_table(table_name)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            let schema = table
                .schema()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            serde_json::to_string(&schema)
                .map_err(|e| PluginError::VectorDb(format!("Failed to serialize schema: {}", e)))
        }

        /// Gets all table names in the database.
        ///
        /// # Returns
        /// List of table names or an error
        pub async fn get_table_names(&self) -> Result<Vec<String>, PluginError> {
            self.conn
                .table_names()
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))
        }

        /// Creates a new table with the provided schema.
        ///
        /// # Arguments
        /// * `table_name` - Name for the new table
        /// * `schema_json` - JSON string representing the table schema
        ///
        /// # Returns
        /// `true` if table creation was successful, or an error
        pub async fn create_table(
            &self,
            table_name: &str,
            schema_json: &str,
        ) -> Result<bool, PluginError> {
            let schema: Schema = serde_json::from_str(schema_json)
                .map_err(|e| PluginError::VectorDb(format!("Invalid schema JSON: {}", e)))?;
            let schema_arc = Arc::new(schema);

            self.conn
                .create_empty_table(table_name, schema_arc)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            Ok(true)
        }

        /// Adds data to a table, including generating embeddings for one column.
        ///
        /// # Arguments
        /// * `table_name` - Target table name
        /// * `embedding_client` - Client for generating embeddings
        /// * `embedding_model` - Model name to use for embedding generation
        /// * `json_str_columns` - JSON strings representing column data
        /// * `to_embed_column_index` - Index of the column to generate embeddings for
        /// * `upsert_on` - Optional column name to use for upsert operations
        ///
        /// # Returns
        /// `true` if data was added successfully, or an error
        pub async fn add(
            &self,
            table_name: String,
            embedding_client: &mut Client,
            embedding_model: String,
            json_str_columns: Vec<String>,
            to_embed_column_index: u32,
            upsert_on: Option<String>,
        ) -> Result<bool, PluginError> {
            let table = self
                .conn
                .open_table(&table_name)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            let schema = table
                .schema()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            // Extract embedding dimension from schema by looking for a FixedSizeList field. It is NOT the embedding column index,
            // that is the field *to embed*, but in the schema there will be a FixedSizeList field that is the actual embeddings
            let (embedding_index, embedding_field) = schema
                .fields()
                .iter()
                .enumerate()
                .find(|(_, field)| matches!(field.data_type(), DataType::FixedSizeList(_, _)))
                .ok_or_else(|| {
                    PluginError::VectorDb("No embedding field found in schema".to_string())
                })?;

            let embedding_dim = match embedding_field.data_type() {
                DataType::FixedSizeList(_, dims) => *dims,
                _ => {
                    return Err(PluginError::VectorDb(
                        "Embedding field is not a FixedSizeList".to_string(),
                    ))
                }
            };

            // Parse JSON strings into columns
            let mut columns: Vec<Vec<serde_json::Value>> = Vec::new();
            for json_str in json_str_columns {
                let column: Vec<serde_json::Value> =
                    serde_json::from_str(&json_str).map_err(|e| {
                        PluginError::VectorDb(format!("Failed to parse JSON column: {}", e))
                    })?;
                columns.push(column);
            }

            // columns are now:
            //  [
            //    [
            //      Value::String("alice"),
            //      Value::String("bob")
            //    ],
            //    [
            //      Value::Number(100),
            //      Value::Number(200),
            //    ]
            //  ]

            // Ensure all columns have the same length
            let row_count = columns.first().map_or(0, |col| col.len());
            for (i, col) in columns.iter().enumerate() {
                if col.len() != row_count {
                    return Err(PluginError::VectorDb(format!(
                        "Column {} has different length than other columns",
                        i
                    )));
                }
            }

            // Generate embeddings for the column at to_embed_column_index
            let embedding_inputs: Vec<String> = columns[to_embed_column_index as usize]
                .iter()
                .map(|text| match text {
                    serde_json::Value::String(t) => Ok(t.clone()),
                    _ => Err(PluginError::VectorDb(
                        "Embedding column contains non-string value".to_string(),
                    )),
                })
                .collect::<Result<Vec<String>, PluginError>>()?;

            let embeddings = if !embedding_inputs.is_empty() {
                embeddings_create(
                    embedding_client,
                    embedding_model,
                    EmbeddingInput::StrArray(embedding_inputs),
                    None,
                    None,
                    None,
                )
                .await?
            } else {
                vec![]
            };

            let arrow_arrays = json_columns_to_arrow_arrays(
                schema.clone(),
                columns,
                embeddings,
                embedding_index,
                embedding_dim,
            )?;

            // Create a RecordBatch with the arrays
            let batch = RecordBatch::try_new(schema.clone(), arrow_arrays).map_err(|e| {
                PluginError::VectorDb(format!("Failed to create record batch: {}", e))
            })?;

            // Create a RecordBatchIterator with the single batch and the schema
            let new_data = RecordBatchIterator::new(vec![Ok(batch)], schema);

            if let Some(upsert_on) = upsert_on {
                println!("merge insert on: {:?}", upsert_on);
                let mut merge_insert = table.merge_insert(&[&upsert_on]);
                merge_insert
                    .when_matched_update_all(None)
                    .when_not_matched_insert_all();
                merge_insert
                    .execute(Box::new(new_data))
                    .await
                    .map_err(|e| PluginError::VectorDb(e.to_string()))?;
            } else {
                // Add the data to the table
                println!("normal insert");
                table
                    .add(new_data)
                    .execute()
                    .await
                    .map_err(|e| PluginError::VectorDb(e.to_string()))?;
            }

            Ok(true)
        }

        pub async fn delete(
            &self,
            table_name: String,
            predicate: String,
        ) -> Result<bool, PluginError> {
            let table = self
                .conn
                .open_table(&table_name)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            table
                .delete(&*predicate)
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            Ok(true)
        }

        /// Retrieves a single row by ID.
        ///
        /// # Arguments
        /// * `table_name` - Name of the table to query
        /// * `id_column` - Column containing ID values
        /// * `id_value` - ID value to search for
        /// * `fields_returned` - List of fields to include in the result
        ///
        /// # Returns
        /// The row as a JSON string if found, None if not found, or an error
        pub async fn get_row_by_id(
            &self,
            table_name: &str,
            id_column: &str,
            id_value: &str,
            fields_returned: Vec<String>,
        ) -> Result<Option<String>, PluginError> {
            let table = self
                .conn
                .open_table(table_name)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            let columns = Select::Columns(fields_returned);
            let results = table
                .query()
                .only_if(format!("{} = '{}'", id_column, id_value))
                .select(columns)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            if results.is_empty() {
                return Ok(None);
            }

            // Convert the first record batch to JSON
            let json_values: Vec<serde_json::Value> =
                serde_arrow::from_record_batch(&results[0])
                    .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            // Return the first row
            let Some(t) = &json_values.into_iter().next() else {
                return Ok(None);
            };
            Ok(Some(serde_json::to_string(t).unwrap()))
        }

        /// Performs a similarity search against vector embeddings.
        ///
        /// # Arguments
        /// * `resources` - Resource table for accessing clients
        /// * `search_config` - Configuration for the similarity search
        /// * `embedding_client` - Client resource for generating embeddings
        /// * `embedding_model` - Model name to use for embedding generation
        /// * `table_name` - Name of the table to search
        /// * `input` - Input text to search for similar items
        ///
        /// # Returns
        /// List of similarity responses or an error
        pub async fn similarity_search(
            &mut self,
            resources: &mut ResourceTable,
            search_config: SimilaritySearchConfig,
            embedding_client: Resource<Client>,
            embedding_model: String,
            table_name: String,
            input: String,
        ) -> Result<Vec<SimilarityResponse>, PluginError> {
            let tbl = self
                .conn
                .open_table(&table_name)
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;
            let input = EmbeddingInput::Str(input);

            let client = resources
                .get_mut(&embedding_client)
                .map_err(|e| PluginError::ResourceError(e.to_string()))?;
            let Some(embedding) =
                embeddings_create(client, embedding_model, input, None, None, None)
                    .await?
                    .pop()
            else {
                return Err(PluginError::EmbeddingError(
                    "No embeddings returned".to_string(),
                ));
            };

            let columns = Select::Columns(search_config.fields_returned.clone());

            let query = tbl.query().select(columns);

            let query = if let Some(where_clause) = search_config.where_clause {
                query.only_if(where_clause)
            } else {
                query
            };

            let query = query
                .nearest_to(embedding)
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            let query = if let Some(limit) = search_config.limit {
                query.limit(limit as usize)
            } else {
                query
            };

            let record_batch = query
                .execute()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| PluginError::VectorDb(e.to_string()))?;

            let mut compared_embed_texts: Vec<SimilarityResponse> = Vec::new();
            for item in record_batch {
                let x: Vec<serde_json::Value> = serde_arrow::from_record_batch(&item)
                    .map_err(|e| PluginError::VectorDb(e.to_string()))?;
                let mut columns: Vec<Vec<String>> = Vec::new();
                let column_names = search_config.fields_returned.clone();
                for _ in 0..column_names.len() {
                    columns.push(Vec::new());
                }
                let mut distances = Vec::new();
                for item in x {
                    if let serde_json::Value::Object(obj) = item {
                        // Extract distance
                        if let Some(distance) = obj.get("_distance") {
                            if let Some(d) = distance.as_f64() {
                                // Check if we should exclude results based on distance threshold
                                if let Some(threshold) = search_config.threshold {
                                    if d > threshold {
                                        continue;
                                    }
                                }
                                // Process each field returned in the search config
                                for (i, field_name) in
                                    search_config.fields_returned.iter().enumerate()
                                {
                                    if let Some(value) = obj.get(field_name) {
                                        if let serde_json::Value::String(s) = value {
                                            columns[i].push(s.clone());
                                        } else {
                                            columns[i].push(serde_json::to_string(value).unwrap());
                                        }
                                    }
                                }

                                distances.push(d);
                            }
                        }
                    }
                }
                let sim_resp = SimilarityResponse {
                    columns,
                    column_names,
                    distances,
                };

                compared_embed_texts.push(sim_resp);
            }
            Ok(compared_embed_texts)
        }
    }

    /// Converts JSON column data to Arrow arrays for database operations.
    ///
    /// # Arguments
    /// * `schema` - Arrow schema for the table
    /// * `columns` - Column data as JSON values
    /// * `embeddings` - Vector embeddings for the embedding column
    /// * `embedding_column_index` - Index of the embedding column
    /// * `embedding_dim` - Dimension of the embedding vectors
    ///
    /// # Returns
    /// Arrow arrays ready for database insertion or an error
    pub fn json_columns_to_arrow_arrays(
        schema: Arc<Schema>,
        columns: Vec<Vec<serde_json::Value>>,
        embeddings: Vec<Vec<f32>>,
        embedding_column_index: usize,
        embedding_dim: i32,
    ) -> Result<Vec<ArrayRef>, PluginError> {
        // For the embedding column, create a FixedSizeListArray
        let option_wrapped_embeddings: Vec<_> = embeddings
            .into_iter()
            .map(|vec| Some(vec.into_iter().map(Some).collect::<Vec<_>>()))
            .collect();

        let embeddings_list = Arc::new(
            FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                option_wrapped_embeddings,
                embedding_dim,
            ),
        );

        let mut arrow_arrays: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());
        for (i, field) in schema.fields().iter().enumerate() {
            if i == embedding_column_index {
                arrow_arrays.push(embeddings_list.clone() as ArrayRef);
            } else {
                // Convert to the appropriate Arrow array type based on field data type
                match field.data_type() {
                    DataType::Utf8 => {
                        // Handle string column
                        let string_values: Vec<String> = columns[i]
                            .iter()
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                _ => v.to_string(),
                            })
                            .collect();
                        arrow_arrays.push(Arc::new(StringArray::from(string_values)) as ArrayRef);
                    }
                    DataType::Int8 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_i64() {
                                Some(v) => Ok(v as i8),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<i8>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Int8Array::from(ints)) as ArrayRef);
                    }
                    DataType::Int16 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_i64() {
                                Some(v) => Ok(v as i16),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<i16>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Int16Array::from(ints)) as ArrayRef);
                    }
                    DataType::Int32 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_i64() {
                                Some(v) => Ok(v as i32),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<i32>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Int32Array::from(ints)) as ArrayRef);
                    }
                    DataType::Int64 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_i64() {
                                Some(v) => Ok(v),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<i64>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Int64Array::from(ints)) as ArrayRef);
                    }
                    DataType::UInt8 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_u64() {
                                Some(v) => Ok(v as u8),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an unsigned integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<u8>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(UInt8Array::from(ints)) as ArrayRef);
                    }
                    DataType::UInt16 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_u64() {
                                Some(v) => Ok(v as u16),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an unsigned integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<u16>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(UInt16Array::from(ints)) as ArrayRef);
                    }
                    DataType::UInt32 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_u64() {
                                Some(v) => Ok(v as u32),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an unsigned integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<u32>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(UInt32Array::from(ints)) as ArrayRef);
                    }
                    DataType::UInt64 => {
                        let ints = columns[i]
                            .iter()
                            .map(|v| match v.as_u64() {
                                Some(v) => Ok(v),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected an unsigned integer, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<u64>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(UInt64Array::from(ints)) as ArrayRef);
                    }
                    DataType::Float16 => {
                        let floats = columns[i]
                            .iter()
                            .map(|v| match v.as_f64() {
                                Some(v) => Ok(v as f32), // Convert to f32 since f16 is not natively supported
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected a float, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<f32>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Float32Array::from(floats)) as ArrayRef);
                        // Use Float32Array as fallback
                    }
                    DataType::Float32 => {
                        let floats = columns[i]
                            .iter()
                            .map(|v| match v.as_f64() {
                                Some(v) => Ok(v as f32),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected a float, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<f32>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Float32Array::from(floats)) as ArrayRef);
                    }
                    DataType::Float64 => {
                        let floats = columns[i]
                            .iter()
                            .map(|v| match v.as_f64() {
                                Some(v) => Ok(v),
                                _ => Err(PluginError::VectorDb(format!(
                                    "Expected a float, for field {}, but it was something else",
                                    field.name()
                                ))),
                            })
                            .collect::<Result<Vec<f64>, PluginError>>()?;
                        arrow_arrays.push(Arc::new(Float64Array::from(floats)) as ArrayRef);
                    }
                    // Add more data type conversions as needed
                    _ => {
                        return Err(PluginError::VectorDb(format!(
                            "Unsupported data type for field {}: {:?}",
                            field.name(),
                            field.data_type()
                        )));
                    }
                }
            }
        }
        Ok(arrow_arrays)
    }
}

#[cfg(not(feature = "vectordb"))]
mod nonvdb {
    /// Stub implementation for database connection when vectordb feature is disabled.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct DbConn {
        /// Path to the database file or directory (unused in stub implementation)
        pub path: String,
    }

    impl DbConn {
        /// Creates a new stub database connection.
        ///
        /// # Arguments
        /// * `path` - Path to the database file or directory (unused)
        ///
        /// # Returns
        /// A new stub `DbConn` instance
        pub async fn new(path: String) -> Self {
            Self { path }
        }
    }
}
