use crate::plugin::run_plugin;
use crate::plugin::StringToStringWorker;
use crate::replace;
use crate::routes::PipelineJob;
use crate::select;
use crate::server::SseStreamTx;
use crate::tenant::user::User;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::injector::error::PluginError;

pub type PipelineRegistry = HashMap<String, Pipeline>;
pub type SyncPipelineRegistry = Arc<RwLock<PipelineRegistry>>;
pub type UserToPipelineRef = HashMap<String, HashSet<String>>;
pub type SyncUserToPipelineRef = Arc<RwLock<UserToPipelineRef>>;

/// Execute a pipeline with the given input
///
/// This function handles the actual execution of a pipeline, processing each stage
/// in sequence and managing the accumulation of results.
pub async fn run_pipeline(
    plugin_manager: Arc<RwLock<HashMap<String, StringToStringWorker>>>,
    jobs: Arc<RwLock<Vec<PipelineJob>>>,
    pipeline: Pipeline,
    input: serde_json::Value,
    job_id: Option<&str>,
    user: Option<User>,
    sse_stream_tx: Option<SseStreamTx>,
) -> Result<(StatusCode, serde_json::Value), (StatusCode, String)> {
    tracing::info!("Running pipeline {}...", pipeline.pipeline_id);

    let final_stage_path = pipeline.stages.len() - 1;

    let mut pipeline_accumulator = PipelineAccumulator {
        input,
        ..Default::default()
    };

    for (i, stage) in pipeline.stages.iter().enumerate() {
        match run_pipeline_stage(
            plugin_manager.clone(),
            jobs.clone(),
            stage,
            vec![i],
            &mut pipeline_accumulator,
            job_id,
            user.as_ref(),
            sse_stream_tx.clone(),
        )
        .await
        {
            Ok(()) => (),
            Err(e) => return Err(e),
        }
        if let Some(jid) = job_id {
            if let Some(job) = jobs.write().await.iter_mut().find(|job| job.id == jid) {
                job.stage = i;
            }
        }
    }

    let result = if let Some(ref output_constructor) = pipeline.output_constructor {
        match pipeline.create_output(&pipeline_accumulator, &output_constructor[..]) {
            Ok(output) => output,
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    } else {
        std::mem::take(
            &mut pipeline_accumulator
                .get_path_mut(vec![final_stage_path])
                .unwrap()
                .output,
        )
    };

    tracing::info!(
        "Finished pipeline - result: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );

    Ok((StatusCode::OK, result))
}

/// Execute a single stage of a pipeline
///
/// The function processes a pipeline stage, including any child stages, and
/// updates the pipeline accumulator with the results. It builds a tree structure of:
/// {
///     "input": <user input>,
///     "stages": [{
///             "output": <output>,
///             "children": [ {"output": <output>, "children": []},...]
///         }
///     ]
/// }
#[allow(clippy::too_many_arguments)]
pub async fn run_pipeline_stage(
    plugin_manager: Arc<RwLock<HashMap<String, StringToStringWorker>>>,
    jobs: Arc<RwLock<Vec<PipelineJob>>>,
    stage: &PipelineStage,
    path: Vec<usize>,
    accumulator: &mut PipelineAccumulator,
    job_id: Option<&str>,
    user: Option<&User>,
    sse_stream_tx: Option<SseStreamTx>,
) -> Result<(), (StatusCode, String)> {
    tracing::info!("running pipeline stage: {}", stage.plugin_id);
    let curr_input = match stage.create_call_input(accumulator) {
        Ok(i) => i,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    let output = run_plugin(
        plugin_manager.clone(),
        jobs.clone(),
        stage.plugin_id.to_string(),
        curr_input,
        job_id.map(|id| id.to_string()),
        user.cloned(),
        sse_stream_tx.clone(),
    )
    .await?;
    let stage_output = PipelineStageOutput {
        plugin_id: stage.plugin_id.clone(),
        output,
        children: Vec::new(),
    };
    let mut parent_path = path.clone();
    parent_path.pop();
    if let Some(parent) = accumulator.get_path_mut(parent_path) {
        parent.children.push(stage_output);
    } else {
        accumulator.stages.push(stage_output);
    }

    for (i, child_stage) in stage.children.iter().enumerate() {
        let mut this_child_path = path.clone();
        this_child_path.push(i);
        match Box::pin(run_pipeline_stage(
            plugin_manager.clone(),
            jobs.clone(),
            child_stage,
            this_child_path,
            accumulator,
            job_id,
            user,
            sse_stream_tx.clone(),
        ))
        .await
        {
            Ok(()) => (),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Represents a pipeline of plugins.
///
/// This struct is used to represent a pipeline of plugins, where each plugin is connected to the next one.
/// The pipeline starts with a `start_plugin_id` and has a `child` field that represents the next plugin in the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Pipeline {
    /// Unique identifier for the pipeline
    pub pipeline_id: String,
    /// The ordered stages that make up this pipeline
    pub stages: Vec<PipelineStage>,
    /// Optional mapping to construct the final output from stage results
    pub output_constructor: Option<Vec<(String, String)>>,
}

impl Pipeline {
    /// Collects all plugin IDs used in this pipeline.
    ///
    /// This method gathers all unique plugin IDs from all stages of the pipeline,
    /// sorts them, and eliminates duplicates.
    ///
    /// # Returns
    ///
    /// A sorted vector of unique plugin IDs
    pub fn plugin_ids(&self) -> Vec<String> {
        let mut all_ids = self
            .stages
            .iter()
            .flat_map(|stage| stage.plugin_ids())
            .collect::<Vec<String>>();
        all_ids.sort();
        all_ids.dedup();
        all_ids
    }

    /// Creates the final output of the pipeline based on the output mapping.
    ///
    /// This method constructs a JSON object by extracting values from the accumulator
    /// according to the provided output mapping.
    ///
    /// # Arguments
    ///
    /// * `accumulator` - The accumulated results from all pipeline stages
    /// * `output_mapping` - Pairs of (source JSONPath, target key) for constructing the output
    ///
    /// # Returns
    ///
    /// * `Ok(Value)` - The constructed output as a JSON value
    /// * `Err(PluginError)` - An error if any selector fails to match
    pub fn create_output(
        &self,
        accumulator: &PipelineAccumulator,
        output_mapping: &[(String, String)],
    ) -> Result<Value, PluginError> {
        let mut input = Map::default();
        let accumulator_json = serde_json::to_value(accumulator).unwrap();
        output_mapping
            .iter()
            .try_for_each(|(output_selector, input_target)| {
                let output_values = select(&accumulator_json, output_selector)
                    .map_err(|e| PluginError::Pipeline(e.to_string()))?;
                if output_values.is_empty() {
                    return Err(PluginError::Pipeline(format!(
                        "create_output: Invalid output selector for pipeline - value did not exist: {}",
                        output_selector
                    )));
                }
                let output = if output_values.len() == 1 {
                    output_values[0].clone()
                } else {
                    Value::Array(output_values.into_iter().cloned().collect::<Vec<_>>())
                };
                input.insert(input_target.clone(), output);
                Ok(())
            })?;
        Ok(serde_json::Value::Object(input))
    }
}

/// Represents a single stage in a pipeline.
///
/// A pipeline stage is responsible for processing data through a specific plugin
/// and can have child stages that run after it completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PipelineStage {
    /// Identifier of the plugin to use for this stage
    pub plugin_id: String,
    /// A json representation of a default value for the input of the plugin.
    pub default_input: Value,
    /// This is used to represent a mapping between output and input JSON paths, which is used to connect the output of one plugin to the input of another plugin.
    ///
    /// It uses [JSONPath](https://en.wikipedia.org/wiki/JSONPath) for the querying. The first element is the output path, and the second element is the input path.
    pub output_to_input: Vec<(String, String)>,
    /// Child stages that execute after this stage completes
    pub children: Vec<Self>,
}

/// Accumulates the results of pipeline stage execution.
///
/// This structure stores both the original input and the outputs from
/// all stages as they are processed through the pipeline.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PipelineAccumulator {
    /// The original input to the pipeline
    pub input: Value,
    /// Results from each stage of the pipeline
    pub stages: Vec<PipelineStageOutput>,
}

impl PipelineAccumulator {
    /// Gets a mutable reference to a specific stage output within the nested structure.
    ///
    /// This method navigates through the tree of stage outputs using a path of indices.
    ///
    /// # Arguments
    ///
    /// * `path` - Vector of indices representing the path to the desired stage output
    ///
    /// # Returns
    ///
    /// * `Some(&mut PipelineStageOutput)` - Reference to the requested stage output
    /// * `None` - If the path is empty or any index is out of bounds
    pub fn get_path_mut(&mut self, mut path: Vec<usize>) -> Option<&mut PipelineStageOutput> {
        if path.is_empty() {
            None
        } else {
            let curr_selector = path.remove(0);
            if let Some(stage) = self.stages.get_mut(curr_selector) {
                Some(stage.get_path_mut(path)?)
            } else {
                None
            }
        }
    }
}

/// Stores the output of a single pipeline stage.
///
/// This structure captures the result of executing a specific plugin
/// along with any results from child stages.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PipelineStageOutput {
    /// ID of the plugin that produced this output
    pub plugin_id: String,
    /// The data produced by the plugin
    pub output: Value,
    /// Outputs from child stages that ran after this one
    pub children: Vec<PipelineStageOutput>,
}

impl PipelineStageOutput {
    /// Gets a mutable reference to a specific stage output within the nested structure.
    ///
    /// This method navigates through the tree of stage outputs using a path of indices.
    ///
    /// # Arguments
    ///
    /// * `path` - Vector of indices representing the path to the desired stage output
    ///
    /// # Returns
    ///
    /// * `Some(&mut Self)` - Reference to the requested stage output
    /// * `None` - If any index in the path is out of bounds
    pub fn get_path_mut(&mut self, mut path: Vec<usize>) -> Option<&mut Self> {
        if path.is_empty() {
            Some(self)
        } else {
            self.children.get_mut(path.remove(0))?.get_path_mut(path)
        }
    }
}

impl PipelineStage {
    /// Collects all plugin IDs used by this stage and its children.
    ///
    /// This method recursively gathers plugin IDs from the entire subtree
    /// starting at this stage.
    ///
    /// # Returns
    ///
    /// A vector of plugin IDs used in this stage and its children
    pub fn plugin_ids(&self) -> Vec<String> {
        let mut ids = vec![self.plugin_id.clone()];
        for child in &self.children {
            ids.extend(child.plugin_ids());
        }
        ids
    }

    /// Creates the input for the plugin based on the accumulated outputs.
    ///
    /// This method constructs an input for the plugin by combining the default input
    /// with values extracted from previous stages according to the output-to-input mapping.
    ///
    /// # Arguments
    ///
    /// * `accumulator` - The accumulated results from previous pipeline stages
    ///
    /// # Returns
    ///
    /// * `Ok(Value)` - The constructed input for the plugin
    /// * `Err(PluginError)` - An error if any selector fails to match
    pub fn create_call_input(
        &self,
        accumulator: &PipelineAccumulator,
    ) -> Result<Value, PluginError> {
        let mut input = self.default_input.clone();
        let accumulator_json = serde_json::to_value(accumulator).unwrap();
        self.output_to_input
            .iter()
            .try_for_each(|(output_selector, input_target)| {
                let output_values = select(&accumulator_json, output_selector)
                    .map_err(|e| PluginError::Pipeline(e.to_string()))?;
                if output_values.is_empty() {
                    return Err(PluginError::Pipeline(format!(
                        "create_call_input: Invalid output selector for pipeline - value did not exist: {}",
                        output_selector
                    )));
                }
                let output = if output_values.len() == 1 {
                    output_values[0].clone()
                } else {
                    Value::Array(output_values.into_iter().cloned().collect::<Vec<_>>())
                };
                input = replace(input.clone(), input_target, output)
                    .map_err(|e| PluginError::Pipeline(e.to_string()))?;
                Ok(())
            })?;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonicalize_json_path;
    use serde_json::json;

    #[test]
    fn test_canonicalize_json_path() {
        assert_eq!(canonicalize_json_path("$.store.book"), "$.store.book");
        assert_eq!(canonicalize_json_path(".store.book"), "$.store.book");
    }

    #[test]
    fn test_select() {
        let json = json!({
            "store": {
                "book": [
                    { "title": "Book 1", "price": 10 },
                    { "title": "Book 2", "price": 20 }
                ]
            }
        });

        let result = select(&json, "$.store.book[0].title").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], &json!("Book 1"));

        let result = select(&json, "$.store.book[*].title").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], &json!("Book 1"));
        assert_eq!(result[1], &json!("Book 2"));

        let error = select(&json, "$.invalid.path").unwrap_err();
        assert!(error.contains("failed selecting from JSON"));
    }

    #[test]
    fn test_replace() {
        let json = json!({
            "store": {
                "book": [
                    { "title": "Book 1", "price": 10 },
                    { "title": "Book 2", "price": 20 }
                ]
            }
        });

        let result = replace(json.clone(), "$.store.book[0].title", json!("New Title")).unwrap();
        assert_eq!(result["store"]["book"][0]["title"], json!("New Title"));

        let error = replace(json, "$.invalid.path", json!("New Value")).unwrap_err();
        assert!(error.contains("failed replacing in JSON"));
    }
}
