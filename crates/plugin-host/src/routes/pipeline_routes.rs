//! Pipeline management and execution functionality
//!
//! This module provides functionality for managing pipelines that orchestrate the execution of plugins.
//! It includes APIs for registering, listing, and executing pipelines both synchronously and asynchronously.
//!
//! Pipelines are sequences of plugin executions where the output of one plugin can be used as input
//! for subsequent plugins, creating complex workflows.
//!
//! Key functionality:
//! - Pipeline registration and listing
//! - Synchronous and asynchronous pipeline execution
//! - Job status tracking for asynchronous executions
//! - Just-In-Time pipeline execution without prior registration

use crate::injector::error::PluginError;
use crate::pipeline::run_pipeline;
use crate::pipeline::Pipeline;
use crate::server::sse_stream;
use crate::server::AppState;
use crate::server::SseStream;
use axum::extract::Query;
use axum::http::HeaderValue;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

/// Represents a pipeline execution job
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PipelineJob {
    /// Unique identifier for the job
    pub id: String,
    /// When the job started
    #[serde(skip)]
    pub start_time: Option<std::time::Instant>,
    /// When the job completed
    #[serde(skip)]
    pub end_time: Option<std::time::Instant>,
    /// Name of the pipeline being executed
    pub pipeline_name: String,
    /// Current stage index of execution
    pub stage: usize,
    /// Total number of stages in the pipeline
    pub total_stages: usize,
    /// Current status message
    pub status: Option<String>,
    /// Final result or error
    pub result: Result<Option<serde_json::Value>, String>,
}

/// Response returned when a pipeline is successfully registered
#[derive(Serialize, Deserialize)]
pub struct RegisterPipelineResponse {
    /// Whether the registration was successful
    pub success: bool,
    /// The unique identifier for the registered pipeline
    pub id: String,
}

/// Structure for a Just-In-Time (JIT) pipeline execution request
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JITPipeline {
    /// The pipeline configuration to execute
    pub pipeline: Pipeline,
    /// The input data to pass to the pipeline
    pub input: serde_json::Value,
}

/// List all registered pipelines
///
/// Returns a list of all pipelines that have been registered with the server,
/// including their complete configuration.
pub async fn pipelines_list(State(state): State<AppState>) -> impl IntoResponse {
    tracing::info!("Request for pipelines list");
    let pipeline_manager = state.pipeline_manager.read().await;

    let pipelines: Vec<(String, Pipeline)> = pipeline_manager
        .iter()
        .map(|(id, pipeline)| (id.clone(), pipeline.clone()))
        .collect();

    (StatusCode::OK, Json(pipelines)).into_response()
}

/// Register a new pipeline
///
/// This endpoint accepts a pipeline configuration and adds it to the
/// internal registry for later execution. It validates that all plugins
/// referenced in the pipeline are available.
pub async fn pipelines_register(
    State(state): State<AppState>,
    Json(pipeline): Json<Pipeline>,
) -> impl IntoResponse {
    tracing::info!("Registering new pipeline");
    let plugin_manager = state.plugin_manager.read().await;

    // Verify all plugins exist
    for plugin_id in &pipeline.plugin_ids() {
        if !plugin_manager.contains_key(plugin_id) {
            tracing::warn!("No plugin found for {plugin_id}");
            return (
                StatusCode::BAD_REQUEST,
                format!("Plugin not found: {}", plugin_id),
            )
                .into_response();
        }
    }

    // Store pipeline
    let mut pipeline_manager = state.pipeline_manager.write().await;
    let pipeline_id = pipeline.pipeline_id.clone();
    pipeline_manager.insert(pipeline_id.clone(), pipeline);

    let response = RegisterPipelineResponse {
        success: true,
        id: pipeline_id,
    };

    (StatusCode::CREATED, Json(response)).into_response()
}

/// Execute a pipeline synchronously
///
/// This endpoint runs a pipeline that has already been registered with the server,
/// using the provided input. The pipeline is identified by its unique ID, and the
/// request waits for the pipeline to complete before returning the result.
pub async fn pipelines_execute(
    State(state): State<AppState>,
    Path(pipeline_id): Path<String>,
    Query(sse): Query<SseStream>,
    input: String,
) -> impl IntoResponse {
    tracing::info!("Request to run pipeline {pipeline_id}");
    // Parse input as JSON
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&input) else {
        tracing::warn!("Bad input json for pipeline {pipeline_id}");
        return (StatusCode::BAD_REQUEST, "Invalid JSON input".to_string()).into_response();
    };

    let pipeline_manager = state.pipeline_manager.read().await;
    let Some(pipeline) = pipeline_manager.get(&pipeline_id).cloned() else {
        tracing::warn!("No pipeline found for {pipeline_id}");
        return (
            StatusCode::NOT_FOUND,
            format!("Pipeline not found: {}", pipeline_id),
        )
            .into_response();
    };
    drop(pipeline_manager);

    let (tx, rx) = if sse.stream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<
            Result<axum::response::sse::Event, PluginError>,
        >();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let fut = run_pipeline(
        state.plugin_manager.clone(),
        state.jobs.clone(),
        pipeline,
        input,
        None,
        None,
        tx,
    );
    if let Some(rx) = rx {
        let typed_stream = axum::response::sse::Sse::new(sse_stream(rx));
        let _res = tokio::spawn(fut);
        let mut resp = typed_stream.into_response();
        resp.headers_mut()
            .append("X-Accel-Buffering", HeaderValue::from_static("no"));
        resp
    } else {
        match fut.await {
            Ok((status, output)) => {
                tracing::info!("Pipeline {pipeline_id} executed successfully");
                (status, Json(output)).into_response()
            }
            Err((status, error)) => {
                tracing::warn!(
                    "Pipeline {pipeline_id} execution failed: {}",
                    error.to_string()
                );
                (status, error).into_response()
            }
        }
    }
}

/// Execute a pipeline asynchronously
///
/// This endpoint starts a pipeline execution in the background and immediately
/// returns a job ID. The client can then use the job status endpoint to monitor
/// the progress and get the result when ready.
pub async fn pipelines_async_execute(
    State(state): State<AppState>,
    Path(pipeline_id): Path<String>,
    input: String,
) -> impl IntoResponse {
    let jobs = state.jobs.clone();
    let job_id = uuid::Uuid::new_v4().to_string();

    tracing::info!("Request to run async pipeline {pipeline_id}");
    // Parse input as JSON
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&input) else {
        tracing::warn!("Bad input json for pipeline {pipeline_id}");
        return (StatusCode::BAD_REQUEST, "Invalid JSON input".to_string()).into_response();
    };

    let pipeline_manager = state.pipeline_manager.read().await;
    let Some(pipeline) = pipeline_manager.get(&pipeline_id).cloned() else {
        tracing::warn!("No pipeline found for {pipeline_id}");
        return (
            StatusCode::NOT_FOUND,
            format!("Pipeline not found: {}", pipeline_id),
        )
            .into_response();
    };
    drop(pipeline_manager);

    {
        jobs.write().await.push(PipelineJob {
            id: job_id.clone(),
            stage: 0,
            total_stages: pipeline.stages.len(),
            start_time: Some(std::time::Instant::now()),
            end_time: None,
            pipeline_name: pipeline_id.clone(),
            status: None,
            result: Ok(None),
        });
    }

    let job_ident = job_id.clone();
    tokio::spawn(async move {
        match run_pipeline(
            state.plugin_manager.clone(),
            jobs.clone(),
            pipeline,
            input,
            Some(&job_ident),
            None,
            None,
        )
        .await
        {
            Ok((_, output)) => {
                tracing::info!("Async pipeline {pipeline_id} executed successfully");
                jobs.write()
                    .await
                    .iter_mut()
                    .find(|job| job.id == job_ident)
                    .map(|job| {
                        job.result = Ok(Some(output));
                        job.end_time = Some(std::time::Instant::now());
                    })
                    .unwrap();
            }
            Err((_, error)) => {
                tracing::warn!(
                    "Async pipeline {pipeline_id} execution failed: {}",
                    error.to_string()
                );
                jobs.write()
                    .await
                    .iter_mut()
                    .find(|job| job.id == job_ident)
                    .map(|job| {
                        job.result = Err(error);
                        job.end_time = Some(std::time::Instant::now());
                    })
                    .unwrap();
            }
        }
    });

    (StatusCode::OK, Json(serde_json::json!({"jobId": job_id}))).into_response()
}

/// Execute a pipeline with Just-In-Time (JIT) compilation
///
/// This endpoint accepts a complete pipeline definition along with input data,
/// and executes it immediately without first registering the pipeline.
pub async fn pipelines_jit_execute(
    State(state): State<AppState>,
    Query(sse): Query<SseStream>,
    Json(input): Json<JITPipeline>,
) -> impl IntoResponse {
    tracing::info!("Running JIT pipeline");
    let pipeline = input.pipeline;

    let (tx, rx) = if sse.stream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<
            Result<axum::response::sse::Event, PluginError>,
        >();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let fut = run_pipeline(
        state.plugin_manager.clone(),
        state.jobs.clone(),
        pipeline,
        input.input,
        None,
        None,
        tx,
    );
    if let Some(rx) = rx {
        let typed_stream = axum::response::sse::Sse::new(sse_stream(rx));
        let _res = tokio::spawn(fut);
        let mut resp = typed_stream.into_response();
        resp.headers_mut()
            .append("X-Accel-Buffering", HeaderValue::from_static("no"));
        resp
    } else {
        match fut.await {
            Ok((status, output)) => (status, Json(output)).into_response(),
            Err((status, error)) => {
                tracing::warn!("Pipeline execution failed: {}", error.to_string());
                (status, error).into_response()
            }
        }
    }
}
