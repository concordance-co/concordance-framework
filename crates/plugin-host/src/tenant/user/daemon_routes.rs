use crate::daemon::{Daemon, DaemonConfig};
use crate::server::AppState;
use crate::tenant::auth::AuthenticatedUser;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use std::collections::HashMap;
use std::sync::Arc;

/// List all daemons belonging to a user
pub async fn user_daemons_list(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> impl IntoResponse {
    tracing::info!("User daemons list requested for: {}", user.user.username);

    let daemons = state
        .tenant_state
        .get_user_daemons(&user.user.username)
        .await;

    let daemons_data = daemons
        .iter()
        .map(|(id, daemon)| {
            serde_json::json!({
                "id": id,
                "name": daemon.config.name,
                "status": daemon.status,
                "last_run": daemon.last_run,
                "next_run": daemon.next_run,
                "frequency_seconds": daemon.config.frequency_seconds
            })
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(daemons_data)).into_response()
}

/// Register a new daemon for a user
pub async fn user_daemon_register(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(payload): Json<DaemonConfig>,
) -> impl IntoResponse {
    let id = payload.name.clone();
    tracing::info!(
        "Registering new daemon '{}' for user: {}",
        id,
        user.user.username
    );

    // Get all plugins visible to the user
    let visible_plugins = state
        .tenant_state
        .get_user_visible_plugins(&user.user.username)
        .await;

    if visible_plugins.is_empty() {
        tracing::warn!("No plugins found for user: {}", user.user.username);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "No plugins found for user"
            })),
        )
            .into_response();
    }

    // Create a plugin registry map for daemon use
    let plugin_map = Arc::new(tokio::sync::RwLock::new(
        visible_plugins
            .iter()
            .filter_map(|(_, worker)| {
                worker.metadata.as_ref().map(|metadata| {
                    let id = crate::injector::metadata_to_id(metadata);
                    (id, worker.clone())
                })
            })
            .collect::<HashMap<_, _>>(),
    ));

    // Create the daemon
    let mut daemon = Daemon::new(id.clone(), payload, plugin_map);
    let state_arc = Arc::new(state.clone());
    daemon.resume(state_arc).await;

    // Register the daemon in the registry and add to user's daemons
    if let Err(e) = state.tenant_state.register_daemon(id.clone(), daemon).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to register daemon: {}", e)
            })),
        )
            .into_response();
    }

    if let Err(e) = state
        .tenant_state
        .add_daemon_to_user(&user.user.username, &id)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to add daemon to user: {}", e)
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "success", "id": id })),
    )
        .into_response()
}

/// Get status of a user's daemon
pub async fn user_daemon_statuses(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(daemon_id): Path<String>,
) -> impl IntoResponse {
    tracing::info!(
        "Daemon status requested for '{}' by user: {}",
        daemon_id,
        user.user.username
    );

    // Check if user has access to this daemon
    let has_access = state
        .tenant_state
        .user_daemons
        .read()
        .await
        .get(&user.user.username)
        .map(|daemon_ids| daemon_ids.contains(&daemon_id))
        .unwrap_or(false);

    if !has_access {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Daemon not found"
            })),
        )
            .into_response();
    }

    // Get daemon from central registry
    let daemon_status = state
        .tenant_state
        .daemon_registry
        .read()
        .await
        .get(&daemon_id)
        .map(|daemon| {
            serde_json::json!({
                "id": daemon_id,
                "name": daemon.config.name,
                "status": daemon.status,
                "last_run": daemon.last_run,
                "next_run": daemon.next_run,
                "error_count": daemon.error_count,
                "last_result": daemon.last_result
            })
        });

    match daemon_status {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Daemon not found"
            })),
        )
            .into_response(),
    }
}

/// Start a user's daemon
pub async fn user_daemon_start(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(daemon_id): Path<String>,
) -> impl IntoResponse {
    tracing::info!(
        "Starting daemon '{}' for user: {}",
        daemon_id,
        user.user.username
    );

    // Check if user has access to this daemon
    let has_access = state
        .tenant_state
        .user_daemons
        .read()
        .await
        .get(&user.user.username)
        .map(|daemon_ids| daemon_ids.contains(&daemon_id))
        .unwrap_or(false);

    if !has_access {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Daemon not found"
            })),
        )
            .into_response();
    }

    // Get and start the daemon from central registry
    let mut daemon_registry = state.tenant_state.daemon_registry.write().await;
    if let Some(daemon) = daemon_registry.get_mut(&daemon_id) {
        let state_arc = Arc::new(state.clone());
        daemon.resume(state_arc).await;

        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": "Daemon started successfully"
            })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "status": "error",
            "message": "Daemon not found in registry"
        })),
    )
        .into_response()
}

/// Stop a user's daemon
pub async fn user_daemon_stop(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(daemon_id): Path<String>,
) -> impl IntoResponse {
    tracing::info!(
        "Stopping daemon '{}' for user: {}",
        daemon_id,
        user.user.username
    );

    // Check if user has access to this daemon
    let has_access = state
        .tenant_state
        .user_daemons
        .read()
        .await
        .get(&user.user.username)
        .map(|daemon_ids| daemon_ids.contains(&daemon_id))
        .unwrap_or(false);

    if !has_access {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Daemon not found"
            })),
        )
            .into_response();
    }

    // Get and stop the daemon from central registry
    let mut daemon_registry = state.tenant_state.daemon_registry.write().await;
    if let Some(daemon) = daemon_registry.get_mut(&daemon_id) {
        daemon.pause();

        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": "Daemon stopped successfully"
            })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "status": "error",
            "message": "Daemon not found in registry"
        })),
    )
        .into_response()
}

/// Share a daemon with an organization
pub async fn share_daemon_with_organization(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path((daemon_id, org_id)): Path<(String, String)>,
) -> impl IntoResponse {
    tracing::info!(
        "Sharing daemon '{}' with organization '{}' by user: {}",
        daemon_id,
        org_id,
        user.user.username
    );

    // Check if user is a member of the organization with at least contributor role
    let has_permission = {
        let orgs = state.tenant_state.organizations.read().await;
        if let Some(org) = orgs.get(&org_id) {
            if let Some(role) = org.member_roles.get(&user.user.username) {
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
                "message": "You must be at least a Contributor to share daemons with this organization"
            })),
        ).into_response();
    }

    // Check if daemon exists in user's personal daemons
    let daemon_exists = {
        let user_daemons = state.tenant_state.user_daemons.read().await;
        if let Some(daemon_ids) = user_daemons.get(&user.user.username) {
            daemon_ids.contains(&daemon_id)
        } else {
            false
        }
    };

    if !daemon_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Daemon not found in your personal daemons"
            })),
        )
            .into_response();
    }

    // Add daemon to organization's daemons
    if let Err(e) = state
        .tenant_state
        .add_daemon_to_org(&org_id, &daemon_id)
        .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to share daemon: {}", e)
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "Daemon shared with organization successfully"
        })),
    )
        .into_response()
}
