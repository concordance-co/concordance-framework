use crate::injector::error::PluginError;
use crate::injector::metadata_to_tool_schema;
use crate::injector::ToolSchema;
use crate::plugin::run_plugin;
use crate::routes::{PluginJit, ServerMetadata, UploadPluginResponse};
use crate::server::sse_stream;
use crate::server::AppState;
use crate::server::SseStream;
use crate::tenant::auth::AuthenticatedUser;
use axum::extract::Query;
use axum::http::HeaderValue;
use axum::{
    body::Bytes,
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use base64::prelude::*;
use reqwest::StatusCode;
use std::collections::HashMap;
use std::sync::Arc; // Add missing Arc import
use tokio::sync::RwLock; // Add missing RwLock import

/// List all personal user registered plugins
///
/// Returns metadata about all plugins that have been registered with the server by this user.
pub async fn user_created_plugins_list(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    tracing::info!(
        "User created plugins list requested for: {}",
        user.user.username
    );

    let created_plugins = state
        .tenant_state
        .get_user_created_plugins(&user.user.username)
        .await;

    let plugins: Vec<ServerMetadata> = created_plugins
        .values()
        .filter_map(|worker| Some(ServerMetadata::from(worker.metadata.as_ref()?)))
        .collect();

    (StatusCode::OK, Json(plugins)).into_response()
}

/// List all plugins visible to the user, including their personal plugins and
/// plugins from all organizations they belong to
///
/// Returns metadata about all plugins that the user has access to.
pub async fn user_all_plugins_list(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    tracing::info!(
        "All visible plugins list requested for: {}",
        user.user.username
    );

    let visible_plugins = state
        .tenant_state
        .get_user_visible_plugins(&user.user.username)
        .await;

    let plugins: Vec<ServerMetadata> = visible_plugins
        .values()
        .filter_map(|worker| Some(ServerMetadata::from(worker.metadata.as_ref()?)))
        .collect();

    (StatusCode::OK, Json(plugins)).into_response()
}

/// Upload a WASM plugin to the server
///
/// This endpoint accepts the raw WASM bytes and adds the plugin
/// to the internal registry for later use.
pub async fn user_plugin_upload(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    wasm_bytes: Bytes,
) -> impl IntoResponse {
    tracing::info!("Request to upload plugin for user: {}", user.user.username);

    // Register the plugin in the central registry
    let (plugin_hash, worker) = match state.tenant_state.register_plugin(&wasm_bytes).await {
        Ok(result) => result,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    // Add the plugin to the user's created plugins
    if let Err(e) = state
        .tenant_state
        .add_user_plugin(
            &user.user.username,
            worker.plugin_id.as_ref().unwrap(),
            &plugin_hash,
        )
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }

    // Get the plugin ID from metadata for the response
    let plugin_id = match worker.metadata.as_ref() {
        Some(metadata) => crate::injector::metadata_to_id(metadata),
        _ => plugin_hash.clone(),
    };

    let plugin_metadata_res = UploadPluginResponse {
        success: true,
        id: plugin_id,
    };

    (StatusCode::CREATED, Json(plugin_metadata_res)).into_response()
}

/// Remove a WASM plugin from the server
///
/// This endpoint accepts the plugin ID and removes the plugin
/// from the internal registry.
pub async fn user_plugin_remove(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    tracing::info!("Request to remove plugin: {}", plugin_id);

    // Find the plugin hash from plugin_id
    let created_plugins = state
        .tenant_state
        .get_user_created_plugins(&user.user.username)
        .await;
    let mut plugin_hash = None;

    for (hash, worker) in created_plugins.iter() {
        if let Some(metadata) = &worker.metadata {
            let id = crate::injector::metadata_to_id(metadata);
            if id == plugin_id {
                plugin_hash = Some(hash.clone());
                break;
            }
        }
    }

    let Some(hash) = plugin_hash else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Plugin not found in your created plugins"
            })),
        )
            .into_response();
    };

    // Remove the plugin from the user's view
    if let Err(e) = state
        .tenant_state
        .remove_user_plugin(&user.user.username, &hash)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to remove plugin: {}", e)
            })),
        )
            .into_response();
    }

    // Run cleanup to remove unused plugins from the registry
    let _ = state.tenant_state.cleanup_unused_plugins().await;

    // Return success response
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "Plugin removed successfully"
        })),
    )
        .into_response()
}

pub async fn user_plugin_execute(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(plugin_id): Path<String>,
    Query(sse): Query<SseStream>,
    input: String,
) -> impl IntoResponse {
    tracing::info!("Request to execute plugin: {}", plugin_id);
    let Ok(input_json) = serde_json::from_str(&input) else {
        return (StatusCode::BAD_REQUEST, "Invalid JSON".to_string()).into_response();
    };

    // Get all plugins visible to the user
    let visible_plugins = state
        .tenant_state
        .get_user_visible_plugins(&user.user.username)
        .await;

    // Find the plugin with this plugin_id
    let mut found_plugin = None;

    for (hash, worker) in visible_plugins.iter() {
        if worker.plugin_id.as_ref().unwrap() == &plugin_id {
            found_plugin = Some(hash.clone());
            break;
        }
    }

    let Some(plugin_hash) = found_plugin else {
        return (
            StatusCode::NOT_FOUND,
            format!("Plugin not found: {}", plugin_id),
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

    let user_clone = user.user.clone();
    let fut = run_plugin(
        state.tenant_state.plugin_registry.clone(),
        state.jobs.clone(),
        plugin_hash,
        input_json,
        None,
        Some(user_clone),
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

pub async fn user_jit_plugin_execute(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(sse): Query<SseStream>,
    Json(input): Json<PluginJit>,
) -> impl IntoResponse {
    tracing::info!(
        "Request to jit execute plugin for user: {}",
        user.user.username
    );
    let wasm_bytes = match BASE64_STANDARD.decode(&input.b64_bytes) {
        Ok(bytes) => bytes,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("Invalid base64: {}", e)).into_response()
        }
    };

    // Register the plugin in the central registry
    let (plugin_hash, worker) = match state.tenant_state.register_plugin(&wasm_bytes).await {
        Ok(result) => result,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    // Add the plugin to the user's created plugins
    if let Err(e) = state
        .tenant_state
        .add_user_plugin(
            &user.user.username,
            worker.plugin_id.as_ref().unwrap(),
            &plugin_hash,
        )
        .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }

    let (tx, rx) = if sse.stream {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<
            Result<axum::response::sse::Event, PluginError>,
        >();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let fut = run_plugin(
        state.tenant_state.plugin_registry.clone(),
        state.jobs.clone(),
        plugin_hash,
        input.input,
        None,
        Some(user.user.clone()),
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

/// Share a plugin with an organization
///
/// This endpoint allows a user to share one of their plugins with an organization they belong to,
/// making it available to all members of that organization.
pub async fn user_share_plugin_with_organization(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path((plugin_id, org_id)): Path<(String, String)>,
) -> impl IntoResponse {
    tracing::info!(
        "Request to share plugin {} with organization {}",
        plugin_id,
        org_id
    );

    // Check if user is a member of the organization with at least contributor role
    let has_permission = {
        let orgs = state.tenant_state.organizations.read().await;
        if let Some(org) = orgs.get(&org_id) {
            if let Some(role) = org.member_roles.get(&user.user.username) {
                // Check if role is at least Contributor
                *role >= crate::tenant::Role::Contributor
            } else {
                false
            }
        } else {
            false
        }
    };

    if !has_permission {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "status": "error",
                "message": "You must be at least a Contributor to share plugins with this organization"
            })),
        ).into_response();
    }

    // Find the plugin hash from plugin_id
    let created_plugins = state
        .tenant_state
        .get_user_created_plugins(&user.user.username)
        .await;
    let mut plugin_hash = None;

    for (hash, worker) in created_plugins.iter() {
        if let Some(metadata) = &worker.metadata {
            let id = crate::injector::metadata_to_id(metadata);
            if id == plugin_id {
                plugin_hash = Some(hash.clone());
                break;
            }
        }
    }

    let Some(hash) = plugin_hash else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Plugin not found in your created plugins"
            })),
        )
            .into_response();
    };

    // Share the plugin with the organization
    if let Err(e) = state
        .tenant_state
        .share_plugin_with_org(&hash, &org_id)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to share plugin: {}", e)
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "Plugin shared with organization successfully"
        })),
    )
        .into_response()
}

/// Return the capabilities supported by the Model Context Protocol (MCP) server
///
/// This endpoint allows clients to discover what tools and integrations
/// are available through the server.
pub async fn user_mcp_capabilities(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    tracing::info!(
        "MCP capabilities requested for user: {}",
        user.user.username
    );

    let visible_plugins = state
        .tenant_state
        .get_user_visible_plugins(&user.user.username)
        .await;

    let tools: Vec<ToolSchema> = visible_plugins
        .values()
        .filter_map(|worker| Some(metadata_to_tool_schema(worker.metadata.as_ref()?)))
        .collect();

    (StatusCode::OK, Json(tools)).into_response()
}
