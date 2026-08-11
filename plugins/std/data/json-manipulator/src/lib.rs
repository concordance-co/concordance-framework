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
use crate::plugin::injector::host::log;
use crate::plugin::injector::logger::Level;

use shared::inlined_schema_for;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Represents the type of operation to perform on JSON data
///
/// Each operation provides different functionality for manipulating JSON data:
/// - Select: Extract data using JSONPath expression
/// - Join: Combine array elements into a string
/// - Filter: Filter array elements based on condition
/// - Sort: Sort array elements by a specified key
/// - Group: Group array elements by a specified key
/// - Transform: Apply template transformation to data
/// - Merge: Combine two JSON structures
/// - Extract: Extract specific properties from objects
/// - Count: Count elements in an array or check existence
/// - Format: Wrap data in an object with specified key
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum OperationType {
    /// Selects data using JSONPath expression
    ///
    /// The source parameter is a JSONPath expression that selects elements from the input data.
    Select {
        /// JSONPath expression to select elements
        source: String,
    },

    /// Joins array elements into a string
    ///
    /// Converts array elements to strings and joins them with a delimiter.
    /// Optionally stores the result in a target location.
    Join {
        /// JSONPath expression that selects an array
        source: String,
        /// Optional JSONPath location to store the result
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Optional delimiter string (defaults to ", ")
        #[serde(skip_serializing_if = "Option::is_none")]
        delimiter: Option<String>,
    },

    /// Filters array elements based on a condition
    ///
    /// The condition is a comparison expression like "@.field > 5" or "@.name == 'value'"
    Filter {
        /// JSONPath expression that selects an array
        source: String,
        /// Filtering condition expression
        condition: String,
    },

    /// Sorts array elements by a specified key
    ///
    /// Orders array elements based on the value of the specified key.
    Sort {
        /// JSONPath expression that selects an array
        source: String,
        /// Key to sort by
        key: String,
        /// Optional sort direction (true for ascending, false for descending)
        #[serde(skip_serializing_if = "Option::is_none")]
        ascending: Option<bool>,
    },

    /// Groups array elements by a specified key
    ///
    /// Creates an object where keys are unique values of the specified property
    /// and values are arrays of elements with that property value.
    Group {
        /// JSONPath expression that selects an array
        source: String,
        /// Key to group by
        key: String,
    },

    /// Transforms data using a template
    ///
    /// Applies a template to each element, where {key} placeholders are replaced by values.
    Transform {
        /// JSONPath expression to select elements for transformation
        source: String,
        /// Template value with {placeholder} patterns
        value: String,
    },

    /// Merges two JSON structures
    ///
    /// Combines objects by merging properties or concatenates arrays.
    Merge {
        /// JSONPath expression for the first data structure
        source: String,
        /// JSONPath expression for the second data structure
        target: String,
    },

    /// Extracts specific properties from objects
    ///
    /// Pulls out a specific property from each element in an array or from an object.
    Extract {
        /// JSONPath expression to select elements
        source: String,
        /// Key to extract from each element
        key: String,
    },

    /// Counts elements in an array or checks existence
    ///
    /// Returns the number of elements in an array or 1/0 for existence of a single element.
    Count {
        /// JSONPath expression to select elements to count
        source: String,
    },

    /// Wraps data in an object with specified key
    ///
    /// Creates a new object with the current data as the value for the specified key.
    Format {
        /// Key name for the resulting object
        key: String,
    },
}

impl Default for OperationType {
    fn default() -> Self {
        OperationType::Select {
            source: "$.".to_string(),
        }
    }
}

/// Request structure for the JSON Path Plus plugin.
/// Contains the input data and a sequence of operations to perform on it.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct JsonPathPlusRequest {
    /// The JSON data to process
    pub data: Value,

    /// Sequence of operations to apply to the data
    pub operations: Vec<OperationType>,
}

/// Response structure returned by the JSON Path Plus plugin
/// Contains the result of applying the operations to the input data
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct JsonPathPlusResponse {
    /// The result of processing the operations on the input data
    pub result: Value,
}

/// A plugin that provides advanced JSON manipulation functionality
/// using JSONPath expressions and various transformation operations.
struct JsonPathPlusPlugin;

impl Guest for JsonPathPlusPlugin {
    type JsonToJson = JsonPathPlusProcessor;

    /// Returns metadata about the plugin
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Json Manipulator".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "A plugin that allows advanced JSON manipulation with JsonPath"
                .to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&inlined_schema_for!(JsonPathPlusRequest)).unwrap(),
            default_input: serde_json::to_string(&JsonPathPlusRequest {
                data: serde_json::Value::Null,
                operations: vec![],
            })
            .unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(JsonPathPlusResponse))
                .unwrap(),
        }
    }
}

/// Processor that implements the JSON manipulation functionality
struct JsonPathPlusProcessor;

impl GuestJsonToJson for JsonPathPlusProcessor {
    /// Main entry point that processes the input JSON string
    ///
    /// # Arguments
    /// * `input` - A JSON string containing the input data and operations
    ///
    /// # Returns
    /// * `Ok(String)` - A JSON string containing the result
    /// * `Err(PluginError)` - An error if processing fails
    fn work(&self, input: String) -> Result<String, PluginError> {
        let request = serde_json::from_str::<JsonPathPlusRequest>(&input)
            .map_err(|e| PluginError::Json(format!("Invalid input JSON: {}", e)))?;

        log(Level::Info, &format!("input: {:#?}", request));

        log(
            Level::Info,
            &format!(
                "Processing input with {} operations",
                request.operations.len()
            ),
        );

        let mut data = request.data.clone();

        for (i, op) in request.operations.iter().enumerate() {
            log(
                Level::Info,
                &format!("Executing operation {}: {:?}", i + 1, op),
            );

            data = match op {
                OperationType::Select { source } => self.handle_select(&data, source),
                OperationType::Join {
                    source,
                    target,
                    delimiter,
                } => self.handle_join(&data, source, target.as_deref(), delimiter.as_deref()),
                OperationType::Filter { source, condition } => {
                    self.handle_filter(&data, source, condition)
                }
                OperationType::Sort {
                    source,
                    key,
                    ascending,
                } => self.handle_sort(&data, source, key, *ascending),
                OperationType::Group { source, key } => self.handle_group(&data, source, key),
                OperationType::Transform { source, value } => {
                    self.handle_transform(&data, source, value)
                }
                OperationType::Merge { source, target } => self.handle_merge(&data, source, target),
                OperationType::Extract { source, key } => self.handle_extract(&data, source, key),
                OperationType::Count { source } => self.handle_count(&data, source),
                OperationType::Format { key } => self.handle_format(&data, key),
            }?;
        }

        serde_json::to_string(&JsonPathPlusResponse { result: data })
            .map_err(|e| PluginError::Json(format!("Failed to serialize response: {}", e)))
    }

    /// Creates a new instance of the processor
    fn new() -> Self {
        Self {}
    }
}

impl JsonPathPlusProcessor {
    /// Helper to safely evaluate a JSONPath expression
    ///
    /// # Arguments
    /// * `data` - The JSON data to evaluate the path against
    /// * `path` - A JSONPath expression string
    ///
    /// # Returns
    /// * Result containing the selected data or an error
    fn evaluate_jsonpath(&self, data: &Value, path: &str) -> Result<Value, PluginError> {
        Ok(jsonpath_lib::select(data, path)
            .map_err(|e| PluginError::Json(format!("Invalid JSONPath: {} - {}", path, e)))?
            .into_iter()
            .cloned()
            .collect())
    }

    /// Converts any JSON value to a string representation for template replacement
    ///
    /// # Arguments
    /// * `value` - JSON value to convert to string
    ///
    /// # Returns
    /// * String representation of the value
    fn value_to_string(&self, value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(_) | Value::Object(_) => {
                // Use serde_json's to_string for complex types, but remove outer quotes
                let json_str = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());

                // If it's a simple value that got quoted, remove the quotes
                if json_str.starts_with('"') && json_str.ends_with('"') && json_str.len() > 1 {
                    json_str[1..json_str.len() - 1].to_string()
                } else {
                    json_str
                }
            }
        }
    }

    /// Processes a string template with values from a JSON object
    ///
    /// Replaces {key} placeholders with corresponding values from the data object
    ///
    /// # Arguments
    /// * `template` - String template with {key} placeholders
    /// * `data` - JSON object containing values to insert
    ///
    /// # Returns
    /// * Processed string with placeholders replaced by values
    fn process_template(&self, template: &str, data: &Value) -> String {
        let mut result = template.to_string();

        if let Some(obj) = data.as_object() {
            for (k, v) in obj {
                let placeholder = format!("{{{}}}", k);
                let value_str = self.value_to_string(v);
                result = result.replace(&placeholder, &value_str);
            }
        }

        result
    }

    /// Handles the Select operation
    ///
    /// Selects data using a JSONPath expression
    ///
    /// # Arguments
    /// * `data` - The JSON data to select from
    /// * `source` - JSONPath expression
    ///
    /// # Returns
    /// * Result containing the selected data or an error
    fn handle_select(&self, data: &Value, source: &str) -> Result<Value, PluginError> {
        self.evaluate_jsonpath(data, source)
    }

    /// Handles the Format operation
    ///
    /// Creates an object with the data as the value for the specified key
    ///
    /// # Arguments
    /// * `data` - The JSON data to wrap
    /// * `key` - The key for the resulting object
    ///
    /// # Returns
    /// * Result containing the formatted object or an error
    fn handle_format(&self, data: &Value, key: &str) -> Result<Value, PluginError> {
        // Create an object with the data as the value for the specified key
        let mut obj = serde_json::Map::new();
        obj.insert(key.to_string(), data.clone());

        Ok(Value::Object(obj))
    }

    /// Handles the Join operation
    ///
    /// Joins array elements into a string with a delimiter
    ///
    /// # Arguments
    /// * `data` - The JSON data containing the array
    /// * `source` - JSONPath expression to select the array
    /// * `target` - Optional path to store the result
    /// * `delimiter` - Optional delimiter string
    ///
    /// # Returns
    /// * Result containing the joined string or an error
    fn handle_join(
        &self,
        data: &Value,
        source: &str,
        target: Option<&str>,
        delimiter: Option<&str>,
    ) -> Result<Value, PluginError> {
        let delimiter = delimiter.unwrap_or(", ");
        let elements = self.evaluate_jsonpath(data, source)?;

        // Convert array elements to strings and join them
        if let Value::Array(arr) = elements {
            if arr.is_empty() {
                log(Level::Warn, "Join operation received an empty array");
                return Ok(Value::String("".to_string()));
            }

            let joined_string = arr
                .iter()
                .map(|v| self.value_to_string(v))
                .collect::<Vec<String>>()
                .join(delimiter);

            // If target is specified, update the original data and return it
            if let Some(target_path) = target {
                let mut result = data.clone();
                self.set_path_value(
                    &mut result,
                    target_path,
                    Value::String(joined_string.clone()),
                )?;
                Ok(result)
            } else {
                // If no target, just return the joined string
                Ok(Value::String(joined_string))
            }
        } else {
            Err(PluginError::Json(
                "Source must evaluate to an array for join operation".to_string(),
            ))
        }
    }

    /// Helper to set a value at a specific JSON path
    ///
    /// # Arguments
    /// * `data` - The JSON data to modify
    /// * `path` - The path where to set the value
    /// * `value` - The value to set
    ///
    /// # Returns
    /// * Result indicating success or an error
    fn set_path_value(
        &self,
        data: &mut Value,
        path: &str,
        value: Value,
    ) -> Result<(), PluginError> {
        let parts: Vec<&str> = path.trim_start_matches('$').split('.').collect();
        let mut current = data;

        for (i, part) in parts.iter().enumerate() {
            let clean_part = part.trim_start_matches('.');
            if clean_part.is_empty() {
                continue;
            }

            // Handle the last part (where we'll set the value)
            if i == parts.len() - 1 {
                if let Some(obj) = current.as_object_mut() {
                    obj.insert(clean_part.to_string(), value.clone());
                } else {
                    return Err(PluginError::Json(
                        "Cannot set target property on non-object".to_string(),
                    ));
                }
            } else {
                // Navigate to the next level
                if let Some(obj) = current.as_object_mut() {
                    if !obj.contains_key(clean_part) {
                        obj.insert(clean_part.to_string(), json!({}));
                    }
                    current = obj.get_mut(clean_part).unwrap();
                } else {
                    return Err(PluginError::Json(format!(
                        "Cannot navigate to '{}' in target path",
                        clean_part
                    )));
                }
            }
        }

        Ok(())
    }

    /// Handles the Filter operation
    ///
    /// Filters array elements based on a condition
    ///
    /// # Arguments
    /// * `data` - The JSON data containing the array
    /// * `source` - JSONPath expression to select the array
    /// * `condition` - Filtering condition expression
    ///
    /// # Returns
    /// * Result containing the filtered array or an error
    fn handle_filter(
        &self,
        data: &Value,
        source: &str,
        condition: &str,
    ) -> Result<Value, PluginError> {
        let elements = self.evaluate_jsonpath(data, source)?;

        if let Value::Array(arr) = elements {
            if arr.is_empty() {
                log(Level::Warn, "Filter operation received an empty array");
                return Ok(Value::Array(vec![]));
            }

            // Improved condition handling with better error reporting
            let filtered: Result<Vec<Value>, PluginError> = arr
                .iter()
                .filter_map(|item| {
                    self.evaluate_condition(item, condition)
                        .map(|matches| if matches { Some(item.clone()) } else { None })
                        .transpose()
                })
                .collect();

            Ok(Value::Array(filtered?))
        } else {
            Err(PluginError::Json(
                "Source must evaluate to an array for filter operation".to_string(),
            ))
        }
    }

    /// Evaluates a condition against an object
    ///
    /// # Arguments
    /// * `item` - The JSON object to evaluate against
    /// * `condition` - Condition expression like "@.field > 5"
    ///
    /// # Returns
    /// * Result containing a boolean indicating if condition is met
    fn evaluate_condition(&self, item: &Value, condition: &str) -> Result<bool, PluginError> {
        // Support for simple comparison operations
        if condition.contains(">") {
            self.evaluate_comparison(item, condition, ">")
        } else if condition.contains("<") {
            self.evaluate_comparison(item, condition, "<")
        } else if condition.contains("==") {
            self.evaluate_comparison(item, condition, "==")
        } else if condition.contains("!=") {
            self.evaluate_comparison(item, condition, "!=")
        } else {
            // For more complex conditions, you'd need a proper expression parser
            Err(PluginError::Json(format!(
                "Unsupported condition: {}",
                condition
            )))
        }
    }

    /// Evaluates a comparison condition
    ///
    /// # Arguments
    /// * `item` - The JSON object to evaluate against
    /// * `condition` - Full condition string
    /// * `operator` - The comparison operator (>, <, ==, !=)
    ///
    /// # Returns
    /// * Result containing a boolean indicating if condition is met
    fn evaluate_comparison(
        &self,
        item: &Value,
        condition: &str,
        operator: &str,
    ) -> Result<bool, PluginError> {
        let parts: Vec<&str> = condition.split(operator).collect();
        if parts.len() != 2 {
            return Err(PluginError::Json(format!(
                "Invalid condition format: {}",
                condition
            )));
        }

        let field_path = parts[0].trim();
        let threshold_str = parts[1].trim();

        // Extract the field name from @.field_name format
        let field_name = field_path.strip_prefix("@.").unwrap_or(field_path);

        // Get the field value
        let field_value = if let Some(obj) = item.as_object() {
            obj.get(field_name).ok_or_else(|| {
                PluginError::Json(format!("Field '{}' not found in object", field_name))
            })?
        } else {
            return Err(PluginError::Json("Item is not an object".to_string()));
        };

        // Parse the threshold value
        let threshold = match field_value {
            Value::Number(_) => threshold_str.parse::<f64>().map_err(|_| {
                PluginError::Json(format!("Cannot parse '{}' as a number", threshold_str))
            })?,
            Value::String(_) => {
                // For string comparison, we'll just use the threshold as is
                return match operator {
                    "==" => Ok(field_value.as_str().unwrap_or("") == threshold_str),
                    "!=" => Ok(field_value.as_str().unwrap_or("") != threshold_str),
                    _ => Err(PluginError::Json(format!(
                        "Operator '{}' not supported for string comparison",
                        operator
                    ))),
                };
            }
            Value::Bool(_) => {
                // For boolean comparison
                let threshold_bool = threshold_str.parse::<bool>().map_err(|_| {
                    PluginError::Json(format!("Cannot parse '{}' as a boolean", threshold_str))
                })?;

                return match operator {
                    "==" => Ok(field_value.as_bool().unwrap_or(false) == threshold_bool),
                    "!=" => Ok(field_value.as_bool().unwrap_or(false) != threshold_bool),
                    _ => Err(PluginError::Json(format!(
                        "Operator '{}' not supported for boolean comparison",
                        operator
                    ))),
                };
            }
            _ => {
                return Err(PluginError::Json(format!(
                    "Comparison not supported for field type: {:?}",
                    field_value
                )));
            }
        };

        // Get the field value as a number
        let field_num = field_value
            .as_f64()
            .ok_or_else(|| PluginError::Json(format!("Field '{}' is not a number", field_name)))?;

        // Compare based on the operator
        match operator {
            ">" => Ok(field_num > threshold),
            "<" => Ok(field_num < threshold),
            "==" => Ok((field_num - threshold).abs() < f64::EPSILON),
            "!=" => Ok((field_num - threshold).abs() >= f64::EPSILON),
            _ => Err(PluginError::Json(format!(
                "Unsupported operator: {}",
                operator
            ))),
        }
    }

    /// Handles the Sort operation
    ///
    /// Sorts array elements by a specified key
    ///
    /// # Arguments
    /// * `data` - The JSON data containing the array
    /// * `source` - JSONPath expression to select the array
    /// * `key` - Key to sort by
    /// * `ascending` - Optional sort direction
    ///
    /// # Returns
    /// * Result containing the sorted array or an error
    fn handle_sort(
        &self,
        data: &Value,
        source: &str,
        key: &str,
        ascending: Option<bool>,
    ) -> Result<Value, PluginError> {
        let ascending = ascending.unwrap_or(true);
        let elements = self.evaluate_jsonpath(data, source)?;

        if let Value::Array(mut arr) = elements {
            if arr.is_empty() {
                log(Level::Warn, "Sort operation received an empty array");
                return Ok(Value::Array(vec![]));
            }

            arr.sort_by(|a, b| {
                let a_val = a.get(key);
                let b_val = b.get(key);

                match (a_val, b_val) {
                    (Some(a_val), Some(b_val)) => {
                        let cmp = match (a_val, b_val) {
                            (Value::String(a_str), Value::String(b_str)) => a_str.cmp(b_str),
                            (Value::Number(a_num), Value::Number(b_num)) => {
                                let a_f = a_num.as_f64().unwrap_or(0.0);
                                let b_f = b_num.as_f64().unwrap_or(0.0);
                                a_f.partial_cmp(&b_f).unwrap_or(std::cmp::Ordering::Equal)
                            }
                            (Value::Bool(a_bool), Value::Bool(b_bool)) => a_bool.cmp(b_bool),
                            _ => {
                                // If types don't match, compare as strings
                                let a_str = self.value_to_string(a_val);
                                let b_str = self.value_to_string(b_val);
                                a_str.cmp(&b_str)
                            }
                        };

                        if ascending {
                            cmp
                        } else {
                            cmp.reverse()
                        }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });

            Ok(Value::Array(arr))
        } else {
            Err(PluginError::Json(
                "Source must evaluate to an array for sort operation".to_string(),
            ))
        }
    }

    /// Handles the Group operation
    ///
    /// Groups array elements by a specified key
    ///
    /// # Arguments
    /// * `data` - The JSON data containing the array
    /// * `source` - JSONPath expression to select the array
    /// * `key` - Key to group by
    ///
    /// # Returns
    /// * Result containing an object with grouped elements or an error
    fn handle_group(&self, data: &Value, source: &str, key: &str) -> Result<Value, PluginError> {
        let elements = self.evaluate_jsonpath(data, source)?;

        if let Value::Array(arr) = elements {
            if arr.is_empty() {
                log(Level::Warn, "Group operation received an empty array");
                return Ok(Value::Object(serde_json::Map::new()));
            }

            let mut groups: serde_json::Map<String, Value> = serde_json::Map::new();

            for item in arr {
                if let Some(group_key) = item.get(key) {
                    let group_key_str = self.value_to_string(group_key);

                    let group = groups.entry(group_key_str).or_insert(Value::Array(vec![]));
                    if let Value::Array(group_arr) = group {
                        group_arr.push(item.clone());
                    }
                } else {
                    log(
                        Level::Warn,
                        &format!("Item does not have the key '{}' for grouping", key),
                    );
                }
            }

            Ok(Value::Object(groups))
        } else {
            Err(PluginError::Json(
                "Source must evaluate to an array for group operation".to_string(),
            ))
        }
    }

    /// Handles the Transform operation
    ///
    /// Transforms elements by applying a template
    ///
    /// # Arguments
    /// * `data` - The JSON data to transform
    /// * `source` - JSONPath expression to select elements
    /// * `value_template` - Template value with placeholders
    ///
    /// # Returns
    /// * Result containing the transformed elements or an error
    fn handle_transform(
        &self,
        data: &Value,
        source: &str,
        template: &str,
    ) -> Result<Value, PluginError> {
        let elements = self.evaluate_jsonpath(data, source)?;

        log(
            Level::Info,
            &format!("Transform source data: {:?}", elements),
        );
        log(Level::Info, &format!("Transform template: {:?}", template));

        if let Value::Array(arr) = &elements {
            if arr.is_empty() {
                log(Level::Warn, "Transform operation received an empty array");
                return Ok(Value::Array(vec![]));
            }

            let mut transformed = Vec::new();

            for item in arr {
                log(Level::Info, &format!("Processing item: {:?}", item));

                let result = self.process_template(template, item);

                // Try to parse as JSON, fall back to string if that fails
                match serde_json::from_str::<Value>(&result) {
                    Ok(json_value) => {
                        log(Level::Info, &format!("Parsed JSON: {:?}", json_value));
                        transformed.push(json_value);
                    }
                    Err(e) => {
                        log(
                            Level::Warn,
                            &format!("Failed to parse as JSON, using as string. Error: {}", e),
                        );
                        transformed.push(Value::String(result));
                    }
                }
            }

            Ok(Value::Array(transformed))
        } else {
            // Single item case
            let result = self.process_template(template, &elements);

            match serde_json::from_str::<Value>(&result) {
                Ok(json_value) => Ok(json_value),
                Err(_) => Ok(Value::String(result)),
            }
        }
    }

    /// Handles the Merge operation
    ///
    /// Merges two JSON structures
    ///
    /// # Arguments
    /// * `data` - The JSON data containing both structures
    /// * `source` - JSONPath expression for first structure
    /// * `target` - JSONPath expression for second structure
    ///
    /// # Returns
    /// * Result containing the merged structure or an error
    fn handle_merge(&self, data: &Value, source: &str, target: &str) -> Result<Value, PluginError> {
        let source_data = self.evaluate_jsonpath(data, source)?;
        let target_data = self.evaluate_jsonpath(data, target)?;

        match (&source_data, &target_data) {
            (Value::Object(source_obj), Value::Object(target_obj)) => {
                let mut result = source_obj.clone();

                // Merge objects
                for (k, v) in target_obj {
                    result.insert(k.clone(), v.clone());
                }

                Ok(Value::Object(result))
            }
            (Value::Array(source_arr), Value::Array(target_arr)) => {
                // Concatenate arrays
                let mut result = source_arr.clone();
                result.extend(target_arr.clone());
                Ok(Value::Array(result))
            }
            _ => {
                // If types don't match, try to convert to most appropriate type
                if let Value::Object(source_obj) = &source_data {
                    let mut result = source_obj.clone();

                    // Try to treat target as an object if possible
                    if let Value::Object(target_obj) = &target_data {
                        for (k, v) in target_obj {
                            result.insert(k.clone(), v.clone());
                        }
                    } else {
                        // Just add target as a special field
                        result.insert("merged_target".to_string(), target_data.clone());
                    }

                    Ok(Value::Object(result))
                } else if let Value::Array(source_arr) = &source_data {
                    // Try to append target to array
                    let mut result = source_arr.clone();

                    if let Value::Array(target_arr) = &target_data {
                        result.extend(target_arr.clone());
                    } else {
                        // Add target as a single element
                        result.push(target_data.clone());
                    }

                    Ok(Value::Array(result))
                } else {
                    Err(PluginError::Json(
                        "Merge operation couldn't reconcile different data types".to_string(),
                    ))
                }
            }
        }
    }

    fn handle_extract(&self, data: &Value, source: &str, key: &str) -> Result<Value, PluginError> {
        let elements = self.evaluate_jsonpath(data, source)?;

        if let Value::Array(arr) = elements {
            if arr.is_empty() {
                log(Level::Warn, "Extract operation received an empty array");
                return Ok(Value::Array(vec![]));
            }

            let extracted: Vec<Value> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(val) = item.get(key) {
                        Some(val.clone())
                    } else {
                        log(Level::Warn, &format!("Key '{}' not found in item", key));
                        None
                    }
                })
                .collect();

            Ok(Value::Array(extracted))
        } else {
            // If not an array, try to extract directly
            if let Some(val) = elements.get(key) {
                Ok(val.clone())
            } else {
                Err(PluginError::Json(format!(
                    "Key '{}' not found in data",
                    key
                )))
            }
        }
    }

    fn handle_count(&self, data: &Value, source: &str) -> Result<Value, PluginError> {
        let elements = self.evaluate_jsonpath(data, source)?;

        if let Value::Array(arr) = elements {
            Ok(json!(arr.len()))
        } else {
            // If not an array, return 1 if it exists, 0 if null
            if elements.is_null() {
                Ok(json!(0))
            } else {
                Ok(json!(1))
            }
        }
    }
}

export!(JsonPathPlusPlugin);
