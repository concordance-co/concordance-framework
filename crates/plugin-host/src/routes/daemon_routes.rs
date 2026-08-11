//! Module for managing background daemon processes that periodically
//! execute pipelines or plugins according to configured schedules.
use crate::daemon::Daemon;
use crate::daemon::DaemonConfig;
use crate::daemon::DaemonStatus;
use crate::routes::AppState;
use axum::http::StatusCode;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use std::sync::Arc;

use tracing::{debug, info, warn};

/// Lightweight status information about a daemon, used for API responses.
#[derive(Debug, Deserialize, Serialize)]
pub struct DaemonStatusInfo {
    /// Unique identifier of the daemon
    pub id: String,
    /// Human-readable name of the daemon
    pub name: String,
    /// Current operational status
    pub status: DaemonStatus,
    /// When the daemon was last executed (if ever)
    pub last_run: Option<chrono::DateTime<chrono::Utc>>,
    /// When the daemon is scheduled to run next
    pub next_run: chrono::DateTime<chrono::Utc>,
    /// Count of consecutive execution errors
    pub error_count: u32,
}

/// API endpoint to list all registered daemons.
///
/// Returns a list of all daemon configurations and their current status.
pub async fn daemon_list(State(state): State<AppState>) -> impl IntoResponse {
    info!("API request: list all daemons");
    let daemons = state.daemons.read().await;
    debug!("Returning {} daemons in response", daemons.len());
    (
        StatusCode::OK,
        Json(daemons.clone().into_iter().collect::<Vec<_>>()),
    )
        .into_response()
}

/// API endpoint to register a new daemon.
///
/// # Arguments
/// * `state` - Application state
/// * `payload` - Daemon configuration
///
/// # Returns
/// JSON response with status and daemon ID
pub async fn daemon_register(
    State(state): State<AppState>,
    Json(payload): Json<DaemonConfig>,
) -> impl IntoResponse {
    let id = payload.name.clone();
    info!(
        daemon_name = %id,
        daemon_type = ?payload.kind,
        frequency = %payload.frequency_seconds,
        "API request: register new daemon"
    );

    let mut daemon = Daemon::new(id.clone(), payload, state.plugin_manager.clone());
    let state_arc = Arc::new(state.clone());
    daemon.resume(state_arc).await;
    state.daemons.write().await.insert(id.clone(), daemon);

    info!(daemon_id = %id, "Daemon registered successfully");
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "success", "id": id })),
    )
        .into_response()
}

/// API endpoint to start/resume a daemon.
///
/// # Arguments
/// * `state` - Application state
/// * `id` - Daemon ID to start
///
/// # Returns
/// Success response or error if daemon not found
pub async fn daemon_start(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(daemon_id = %id, "API request: start/resume daemon");
    let mut daemons = state.daemons.write().await;

    match daemons.get_mut(&id) {
        Some(daemon) => {
            // Clone state for the background task
            let state_arc = Arc::new(state.clone());
            daemon.resume(state_arc).await;

            info!(daemon_id = %id, "Daemon resumed successfully");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "success",
                    "message": "Daemon resumed"
                })),
            )
                .into_response()
        }
        None => {
            warn!(daemon_id = %id, "Attempted to start non-existent daemon");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "Daemon not found"
                })),
            )
                .into_response()
        }
    }
}

/// API endpoint to stop/pause a daemon.
///
/// # Arguments
/// * `state` - Application state
/// * `id` - Daemon ID to stop
///
/// # Returns
/// Success response or error if daemon not found
pub async fn daemon_stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    info!(daemon_id = %id, "API request: stop/pause daemon");
    let mut daemons = state.daemons.write().await;

    match daemons.get_mut(&id) {
        Some(daemon) => {
            daemon.pause();
            info!(daemon_id = %id, "Daemon paused successfully");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "success",
                    "message": "Daemon paused"
                })),
            )
                .into_response()
        }
        None => {
            warn!(daemon_id = %id, "Attempted to stop non-existent daemon");
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "Daemon not found"
                })),
            )
                .into_response()
        }
    }
}

/// API endpoint to get status information for all daemons.
///
/// Returns a condensed view of daemon statuses without full configuration details.
pub async fn daemon_statuses(State(state): State<AppState>) -> impl IntoResponse {
    info!("API request: get status of all daemons");
    let daemons = state.daemons.read().await;

    let statuses: Vec<DaemonStatusInfo> = daemons
        .iter()
        .map(|(id, daemon)| DaemonStatusInfo {
            id: id.clone(),
            name: daemon.config.name.clone(),
            status: daemon.status.clone(),
            last_run: daemon.last_run,
            next_run: daemon.next_run,
            error_count: daemon.error_count,
        })
        .collect();

    debug!("Returning status for {} daemons", statuses.len());
    (StatusCode::OK, Json(statuses)).into_response()
}
