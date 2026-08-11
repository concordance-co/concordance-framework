use crate::injector::error::PluginError;
use crate::pipeline::{run_pipeline, Pipeline};
use crate::routes::PipelineJob;
use crate::server::sse_stream;
use crate::server::{AppState, SseStream};
use crate::tenant::auth::AuthenticatedUser;
use axum::extract::Query;
use axum::http::HeaderValue;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// List all pipelines available to the user
pub async fn user_pipelines_list(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    tracing::info!(
        "User pipelines list requested for user: {}",
        user.user.username
    );

    // Fix the temporary value dropped while borrowed issue by storing the lock
    let user_pipeline_refs = state.tenant_state.user_pipeline_refs.read().await;

    let pipeline_data = if let Some(pipeline_ids) = user_pipeline_refs.get(&user.user.username) {
        // Get pipelines from registry
        let pipeline_registry = state.tenant_state.pipeline_registry.read().await;

        // Map the pipeline IDs to their pipeline data
        pipeline_ids
            .iter()
            .filter_map(|id| pipeline_registry.get(id).map(|pipeline| (id, pipeline)))
            .map(|(id, pipeline)| {
                serde_json::json!({
                    "id": id,
                    "name": pipeline.pipeline_id,
                    "stages": pipeline.stages.len(),
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    (StatusCode::OK, Json(pipeline_data)).into_response()
}

/// Register a new pipeline for the user
pub async fn user_pipelines_register(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(pipeline): Json<Pipeline>,
) -> impl IntoResponse {
    tracing::info!("Registering new pipeline for user: {}", user.user.username);

    // Get the user's visible plugins
    let visible_plugins = state
        .tenant_state
        .get_user_visible_plugins(&user.user.username)
        .await;

    // Verify all plugins exist
    for plugin_id in &pipeline.plugin_ids() {
        let plugin_exists = visible_plugins.iter().any(|(_, worker)| {
            if let Some(metadata) = &worker.metadata {
                let id = crate::injector::metadata_to_id(metadata);
                id == *plugin_id
            } else {
                false
            }
        });

        if !plugin_exists {
            tracing::warn!("No plugin found for {plugin_id}");
            return (
                StatusCode::BAD_REQUEST,
                format!("Plugin not found: {}", plugin_id),
            )
                .into_response();
        }
    }

    // Store pipeline in central registry
    let pipeline_id = pipeline.pipeline_id.clone();
    state
        .tenant_state
        .pipeline_registry
        .write()
        .await
        .insert(pipeline_id.clone(), pipeline);

    // Add pipeline to user's pipeline references
    let mut user_pipeline_refs = state.tenant_state.user_pipeline_refs.write().await;
    let user_pipeline_set = user_pipeline_refs
        .entry(user.user.username.clone())
        .or_insert_with(HashSet::new);
    user_pipeline_set.insert(pipeline_id.clone());

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "id": pipeline_id
        })),
    )
        .into_response()
}

/// Execute a pipeline owned by the user
pub async fn user_pipelines_execute(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(pipeline_id): Path<String>,
    Query(sse): Query<SseStream>,
    input: String,
) -> impl IntoResponse {
    tracing::info!(
        "Request to run pipeline {pipeline_id} for user: {}",
        user.user.username
    );

    // Parse input as JSON
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&input) else {
        tracing::warn!("Bad input json for pipeline {pipeline_id}");
        return (StatusCode::BAD_REQUEST, "Invalid JSON input".to_string()).into_response();
    };

    // Check if pipeline exists in user's pipeline references
    let pipeline_exists = state
        .tenant_state
        .user_pipeline_refs
        .read()
        .await
        .get(&user.user.username)
        .map(|pipeline_ids| pipeline_ids.contains(&pipeline_id))
        .unwrap_or(false);

    if !pipeline_exists {
        tracing::warn!("User doesn't have access to pipeline {pipeline_id}");
        return (
            StatusCode::NOT_FOUND,
            format!("Pipeline not found: {}", pipeline_id),
        )
            .into_response();
    }

    // Get the pipeline from registry
    let pipeline = state
        .tenant_state
        .pipeline_registry
        .read()
        .await
        .get(&pipeline_id)
        .cloned();

    let Some(pipeline) = pipeline else {
        tracing::warn!("No pipeline found for {pipeline_id}");
        return (
            StatusCode::NOT_FOUND,
            format!("Pipeline not found: {}", pipeline_id),
        )
            .into_response();
    };

    let (tx, rx) = if sse.stream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<
            Result<axum::response::sse::Event, PluginError>,
        >();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let user = user.user.clone();
    let fut = run_pipeline(
        state.tenant_state.plugin_registry.clone(),
        state.jobs.clone(),
        pipeline,
        input,
        None,
        Some(user),
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

/// Share a pipeline with an organization
pub async fn share_pipeline_with_organization(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path((pipeline_id, organization_id)): Path<(String, String)>,
) -> impl IntoResponse {
    tracing::info!(
        "Sharing pipeline {} with organization {} by user {}",
        pipeline_id,
        organization_id,
        user.user.username
    );

    // Check if the organization exists
    let organizations = state.tenant_state.organizations.read().await;
    let org = match organizations.get(&organization_id) {
        Some(org) => org,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Organization not found"
                })),
            )
                .into_response();
        }
    };

    // Check if user is a member of the organization with at least Contributor role
    if !org.member_ids.contains(&user.user.username) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "You are not a member of this organization"
            })),
        )
            .into_response();
    }

    // Check if user has sufficient permissions (Contributor or higher)
    let user_role = org
        .member_roles
        .get(&user.user.username)
        .cloned()
        .unwrap_or(crate::tenant::Role::Viewer);
    if user_role < crate::tenant::Role::Contributor {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "You need at least Contributor role to share pipelines with this organization"
            })),
        )
            .into_response();
    }

    // Check if pipeline exists in user's pipeline references
    let pipeline_exists = state
        .tenant_state
        .user_pipeline_refs
        .read()
        .await
        .get(&user.user.username)
        .map(|pipeline_ids| pipeline_ids.contains(&pipeline_id))
        .unwrap_or(false);

    if !pipeline_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Pipeline not found in your pipelines"
            })),
        )
            .into_response();
    }

    // Add the pipeline to the organization's pipeline references
    let mut org_pipelines = state.tenant_state.org_pipelines.write().await;
    let org_pipeline_set = org_pipelines
        .entry(organization_id.clone())
        .or_insert_with(HashSet::new);
    org_pipeline_set.insert(pipeline_id.clone());

    // Return success
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Pipeline shared successfully with organization"
        })),
    )
        .into_response()
}

/// Execute a pipeline asynchronously for the user
///
/// This endpoint starts a pipeline execution in the background and immediately
/// returns a job ID. The client can then use the job status endpoint to monitor
/// the progress and get the result when ready.
pub async fn user_pipelines_async_execute(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(pipeline_id): Path<String>,
    input: String,
) -> impl IntoResponse {
    let jobs = state.jobs.clone();
    let job_id = uuid::Uuid::new_v4().to_string();

    tracing::info!(
        "Request to run async pipeline {pipeline_id} for user: {}",
        user.user.username
    );
    // Parse input as JSON
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&input) else {
        tracing::warn!("Bad input json for pipeline {pipeline_id}");
        return (StatusCode::BAD_REQUEST, "Invalid JSON input".to_string()).into_response();
    };

    // Check if pipeline exists in user's pipeline references
    let pipeline_exists = state
        .tenant_state
        .user_pipeline_refs
        .read()
        .await
        .get(&user.user.username)
        .map(|pipeline_ids| pipeline_ids.contains(&pipeline_id))
        .unwrap_or(false);

    if !pipeline_exists {
        tracing::warn!("User doesn't have access to pipeline {pipeline_id}");
        return (
            StatusCode::NOT_FOUND,
            format!("Pipeline not found: {}", pipeline_id),
        )
            .into_response();
    }

    // Get the pipeline from registry
    let pipeline = state
        .tenant_state
        .pipeline_registry
        .read()
        .await
        .get(&pipeline_id)
        .cloned();

    let Some(pipeline) = pipeline else {
        tracing::warn!("No pipeline found for {pipeline_id}");
        return (
            StatusCode::NOT_FOUND,
            format!("Pipeline not found: {}", pipeline_id),
        )
            .into_response();
    };

    // Get the user's visible plugins
    let visible_plugins = state
        .tenant_state
        .get_user_visible_plugins(&user.user.username)
        .await;

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
            state.tenant_state.plugin_registry.clone(),
            jobs.clone(),
            pipeline,
            input,
            Some(&job_ident),
            Some(user.user.clone()),
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
