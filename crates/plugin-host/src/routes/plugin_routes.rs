//! Plugin management API routes
//!
//! This module contains handlers for plugin operations including:
//! - Uploading plugins
//! - Executing plugins (both pre-registered and JIT compilation)
//! - Listing available plugins

use crate::injector::error::PluginError;
use crate::plugin::new_worker;
use crate::plugin::run_plugin;
use crate::routes::{AppState, ServerMetadata};
use crate::server::sse_stream;
use crate::server::SseStream;
use axum::extract::Query;
use axum::http::HeaderValue;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::prelude::*;
use serde::{Deserialize, Serialize};

/// Response returned when a plugin is successfully uploaded
#[derive(Serialize, Deserialize)]
pub struct UploadPluginResponse {
    /// Whether the upload was successful
    pub success: bool,
    /// The unique identifier for the uploaded plugin
    pub id: String,
}

/// Structure for just-in-time plugin execution requests
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginJit {
    /// The input data to pass to the plugin
    pub input: serde_json::Value,
    /// Base64-encoded WASM bytes of the plugin
    pub b64_bytes: String,
}

/// Upload a WASM plugin to the server
///
/// This endpoint accepts the raw WASM bytes and adds the plugin
/// to the internal registry for later use.
pub async fn plugin_upload(State(state): State<AppState>, wasm_bytes: Bytes) -> impl IntoResponse {
    tracing::info!("Request to upload plugin");
    let (plugin_id, worker) = match new_worker(
        &wasm_bytes[..],
        state.plugin_manager.clone(),
        Default::default(),
        Default::default(),
    )
    .await
    {
        Ok((plugin_id, worker)) => (plugin_id, worker),
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let mut manager = state.plugin_manager.write().await;
    manager.insert(plugin_id.clone(), worker);

    let plugin_metadata_res = UploadPluginResponse {
        success: true,
        id: plugin_id,
    };

    (StatusCode::CREATED, Json(plugin_metadata_res)).into_response()
}

/// Execute a plugin with Just-In-Time (JIT) compilation
///
/// This endpoint accepts a base64-encoded WASM plugin, decodes it,
/// loads it into the plugin registry, and executes it with the provided input.
#[axum::debug_handler]
pub async fn jit_plugin_execute(
    State(state): State<AppState>,
    Query(sse): Query<SseStream>,
    Json(input): Json<PluginJit>,
) -> impl IntoResponse {
    tracing::info!("Request to jit execute plugin");
    let wasm_bytes = BASE64_STANDARD.decode(input.b64_bytes).unwrap();

    let (plugin_id, worker) = match new_worker(
        &wasm_bytes[..],
        state.plugin_manager.clone(),
        Default::default(),
        Default::default(),
    )
    .await
    {
        Ok((plugin_id, worker)) => (plugin_id, worker),
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let mut manager = state.plugin_manager.write().await;
    manager.insert(plugin_id.clone(), worker);
    drop(manager);

    let (tx, rx) = if sse.stream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<
            Result<axum::response::sse::Event, PluginError>,
        >();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let fut = run_plugin(
        state.plugin_manager.clone(),
        state.jobs.clone(),
        plugin_id,
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
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err((s, e)) => {
                tracing::warn!("Plugin execution failed: {}", e.to_string());
                (s, e).into_response()
            }
        }
    }
}

/// Execute a specific plugin by ID
///
/// This endpoint runs a plugin that has already been registered with the server,
/// using the provided input. The plugin is identified by its unique ID.
#[axum::debug_handler]
pub async fn plugin_execute(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(sse): Query<SseStream>,
    input: String,
) -> impl IntoResponse {
    tracing::info!("Request to execute plugin");
    let Ok(val) = serde_json::from_str(&input) else {
        return (StatusCode::BAD_REQUEST, "Invalid JSON".to_string()).into_response();
    };

    let (tx, rx) = if sse.stream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<
            Result<axum::response::sse::Event, PluginError>,
        >();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let fut = run_plugin(
        state.plugin_manager.clone(),
        state.jobs.clone(),
        plugin_id,
        val,
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
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err((s, e)) => {
                tracing::warn!("Plugin execution failed: {}", e.to_string());
                (s, e).into_response()
            }
        }
    }
}

/// List all registered plugins
///
/// Returns metadata about all plugins that have been registered with the server.
pub async fn plugins_list(State(state): State<AppState>) -> impl IntoResponse {
    tracing::info!("Plugins list requested");
    let plugin_manager = state.plugin_manager.read().await;

    let plugins: Vec<ServerMetadata> = plugin_manager
        .iter()
        .filter_map(|(_, worker)| Some(ServerMetadata::from(worker.metadata.as_ref()?)))
        .collect();

    (StatusCode::OK, Json(plugins)).into_response()
}
