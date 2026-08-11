mod daemon_routes;
mod pipeline_routes;
mod plugin_routes;

pub use daemon_routes::*;
pub use pipeline_routes::*;
pub use plugin_routes::*;

use crate::injector::{
    exports::plugin::injector::guest::PluginKind, metadata_to_id, metadata_to_tool_schema,
    Metadata, ToolSchema,
};
use crate::server::AppState;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

/// Metadata about a plugin for API responses
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ServerMetadata {
    /// Unique identifier for the plugin
    pub id: String,
    /// Display name of the plugin
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Description of what the plugin does
    pub description: String,
    /// Author of the plugin
    pub author: String,
    /// Type of plugin (transformer, etc.)
    pub kind: PluginKind,
    /// Default input values
    pub default_input: serde_json::Value,
    /// JSON schema for the input
    pub input_schema: schemars::Schema,
    /// JSON schema for the output
    pub output_schema: schemars::Schema,
}

impl From<&Metadata> for ServerMetadata {
    fn from(metadata: &Metadata) -> Self {
        ServerMetadata {
            id: metadata_to_id(metadata),
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            description: metadata.description.clone(),
            author: metadata.author.clone(),
            kind: metadata.kind,
            default_input: serde_json::to_value(&metadata.default_input).unwrap(),
            input_schema: serde_json::from_str(&metadata.input_schema).unwrap(),
            output_schema: serde_json::from_str(&metadata.output_schema).unwrap(),
        }
    }
}

/// Get the status of a specific job
///
/// This endpoint allows clients to check on the progress of an asynchronous
/// pipeline execution, including whether it's completed and its results.
pub async fn job_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    tracing::info!("Job status requested for {job_id}");
    let status = {
        let jobs = state.jobs.read().await;
        jobs.iter().find(|job| job.id == job_id).map(|job| {
            serde_json::json!({
                "status": job.status.clone(),
                "result": match &job.result {
                    Ok(result) => result.clone(),
                    Err(err) => Some(serde_json::to_value(err.to_string()).unwrap()),
                },
                "stage": job.stage.clone(),
                "totalStages": job.total_stages.clone(),
            })
        })
    };

    if let Some(status) = status {
        // Return the job status as JSON
        (StatusCode::OK, Json(status)).into_response()
    } else {
        // Job not found
        (StatusCode::NOT_FOUND, "Job not found".to_string()).into_response()
    }
}

/// Return the capabilities supported by the Model Context Protocol (MCP) server
///
/// This endpoint allows clients to discover what tools and integrations
/// are available through the server.
pub async fn mcp_capabilities(State(state): State<AppState>) -> impl IntoResponse {
    tracing::info!("MCP capabilities requested");

    let tools: Vec<ToolSchema> = state
        .plugin_manager
        .read()
        .await
        .iter()
        .filter_map(|(_, worker)| Some(metadata_to_tool_schema(worker.metadata.as_ref()?)))
        .collect();
    (StatusCode::OK, Json(tools)).into_response()
}
