//! Plugin server module for Concordance.
//!
//! This module provides a web server that can load, manage and execute WASM plugins
//! and pipelines. It handles plugin registration, execution, and status tracking.

use crate::daemon::Daemon;
use crate::injector::error::PluginError;
use crate::injector::metadata_to_id;
use crate::persistence::{load_state, persistence_middleware, PersistenceService};
use crate::pipeline::Pipeline;
use crate::plugin::StringToStringWorker;
use crate::routes::PipelineJob;
use crate::routes::{
    daemon_list, daemon_register, daemon_start, daemon_statuses, daemon_stop, jit_plugin_execute,
    job_status, mcp_capabilities, pipelines_async_execute, pipelines_execute,
    pipelines_jit_execute, pipelines_list, pipelines_register, plugin_execute, plugin_upload,
    plugins_list,
};
use std::path::PathBuf;

use anyhow::Result;
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use futures_core::TryStream;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use crate::tenant::TenantState;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub type SseStreamTx = UnboundedSender<Result<axum::response::sse::Event, PluginError>>;
pub type SseStreamRx = UnboundedReceiver<Result<axum::response::sse::Event, PluginError>>;

pub fn sse_stream(
    mut rx: SseStreamRx,
) -> impl TryStream<
    Item = Result<axum::response::sse::Event, PluginError>,
    Ok = axum::response::sse::Event,
    Error = PluginError,
> {
    async_stream::try_stream! {
        while let Some(received) = rx.recv().await {
            let event: axum::response::sse::Event = received?;
            // tracing::info!("Received event: {:?}", event);
            yield event;
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SseStream {
    #[serde(default)]
    pub stream: bool,
}

/// Application state containing shared resources
#[derive(Clone, Debug)]
pub struct AppState {
    /// Registry of available plugins
    pub plugin_manager: Arc<RwLock<HashMap<String, StringToStringWorker>>>,
    /// Registry of available pipelines
    pub pipeline_manager: Arc<RwLock<HashMap<String, Pipeline>>>,
    /// List of ongoing and completed jobs
    pub jobs: Arc<RwLock<Vec<PipelineJob>>>,
    /// Daemons
    pub daemons: Arc<RwLock<HashMap<String, Daemon>>>,
    /// Tenant state for multi-user support
    pub tenant_state: TenantState,
    /// Persistence service
    pub persistence: PersistenceService,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            plugin_manager: Default::default(),
            pipeline_manager: Default::default(),
            jobs: Default::default(),
            daemons: Default::default(),
            tenant_state: Default::default(),
            persistence: PersistenceService::new(
                home::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".concordance"),
                "state.json",
            ),
        }
    }
}

pub fn add_routes(router: Router<AppState>, with_auth: bool) -> Router<AppState> {
    if with_auth {
        use crate::tenant::{auth::api_key::*, org::*, user::*};
        router
            // --- Organization management routes ---
            .route("/org/register", post(org_register))
            .route("/org/{id}/set-env-var", post(org_add_env_var))
            .route("/org/{id}/add-user", post(org_add_user))
            .route("/org/{id}/info", get(org_info))
            .route("/org/{id}/users/list", get(org_users_list))
            .route("/org/{id}/plugins/list", get(org_plugins_list))
            .route("/org/{id}/pipelines/list", get(org_pipelines_list))
            .route("/org/{id}/daemons/list", get(org_daemons_list))
            // --- User-specific routes ---
            // --- auth ---
            .route("/register", post(user_register))
            .route("/login", post(user_login))
            .route("/create-api-key", post(create_api_key))
            .route("/api-keys", get(list_api_keys))
            .route("/remove-api-key", post(remove_api_key))
            // --- config ---
            .route("/set-env-var", post(user_add_env_var))
            .route("/info", get(user_info))
            // --- plugins ---
            .route("/plugins/all", get(user_all_plugins_list))
            .route("/plugins/created", get(user_created_plugins_list))
            .route("/plugins/upload", post(user_plugin_upload))
            .route("/plugins/jit-execute", post(user_jit_plugin_execute))
            .route("/plugins/{id}/remove", post(user_plugin_remove))
            .route("/plugins/{id}/execute", post(user_plugin_execute))
            .route(
                "/plugins/{id}/share/{org}",
                post(user_share_plugin_with_organization),
            )
            // --- pipelines ---
            .route("/pipelines/list", get(user_pipelines_list))
            .route("/pipelines/register", post(user_pipelines_register))
            .route("/pipelines/{id}/execute", post(user_pipelines_execute))
            .route(
                "/pipelines/{id}/async-execute",
                post(user_pipelines_async_execute),
            )
            .route(
                "/pipelines/{id}/share/{org}",
                post(share_pipeline_with_organization),
            )
            // --- daemons ---
            .route("/daemons/list", get(user_daemons_list))
            .route("/daemons/register", post(user_daemon_register))
            .route("/daemons/{id}/statuses", post(user_daemon_statuses))
            .route("/daemons/{id}/start", post(user_daemon_start))
            .route("/daemons/{id}/stop", post(user_daemon_stop))
            .route(
                "/daemons/{id}/share/{org}",
                post(share_daemon_with_organization),
            )
            // --- misc ---
            .route("/jobs/{id}", get(job_status))
            .route("/mcp/capabilities", get(user_mcp_capabilities))
    } else {
        router
            // --- pipelines ---
            .route("/pipelines/list", get(pipelines_list))
            .route("/pipelines/register", post(pipelines_register))
            .route("/pipelines/jit-execute", post(pipelines_jit_execute))
            .route("/pipelines/{id}/execute", post(pipelines_execute))
            .route(
                "/pipelines/{id}/async-execute",
                post(pipelines_async_execute),
            )
            // --- daemons ---
            .route("/daemons/list", get(daemon_list))
            .route("/daemons/register", post(daemon_register))
            .route("/daemons/statuses", get(daemon_statuses))
            .route("/daemons/{id}/start", post(daemon_start))
            .route("/daemons/{id}/stop", post(daemon_stop))
            // --- plugins ---
            .route("/plugins/list", get(plugins_list))
            .route("/plugins/upload", post(plugin_upload))
            .route("/plugins/jit-execute", post(jit_plugin_execute))
            .route("/plugins/{id}/execute", post(plugin_execute))
            // --- misc ---
            .route("/jobs/{id}", get(job_status))
            .route("/mcp/capabilities", get(mcp_capabilities))
    }
}

/// Start the server with the specified configuration
///
/// This function initializes the plugin server, sets up routes for the API,
/// loads initial plugins, and begins listening for requests.
pub async fn start_server(
    addr: impl Into<SocketAddr>,
    injector_plugins: Vec<String>,
    jwt_secret: Option<String>,
) -> Result<()> {
    let addr: SocketAddr = addr.into();
    tracing::info!("Starting server at {}", addr.clone());

    // Create the application state
    let mut app_state = AppState::default();
    let with_auth = jwt_secret.is_some();
    if let Some(secret) = jwt_secret {
        app_state.tenant_state = TenantState::new(Some(secret));
    }

    let _ = load_state(&app_state.persistence, app_state.clone()).await;

    let jobs = app_state.jobs.clone();
    let pm = app_state.plugin_manager.clone();

    // Define our routes
    let app = add_routes(Router::new(), with_auth)
        .with_state(app_state.clone())
        .layer(DefaultBodyLimit::disable()) // TODO: Find a good size limit for plugin uploads
        .layer(CorsLayer::permissive()) // Add CORS support
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            persistence_middleware,
        ));

    // Set up the server
    tokio::spawn(async move {
        // Process each plugin sequentially
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        for file in injector_plugins {
            // Read the file as bytes
            let wasm_bytes = std::fs::read(&file)
                .unwrap_or_else(|_| panic!("Failed to read plugin file: {}", file));

            // Create the worker from bytes
            match StringToStringWorker::from_bytes(
                pm.clone(),
                Default::default(),
                Default::default(),
                &wasm_bytes,
            )
            .await
            {
                Ok(worker) => {
                    let plugin_id = metadata_to_id(worker.metadata.as_ref().unwrap());
                    tracing::info!("Added initial plugin {plugin_id} to server");
                    pm.write().await.insert(plugin_id, worker);
                }
                Err(e) => {
                    tracing::warn!("Failed to add initial plugin to server: {e}");
                    println!(
                        "Failed to create worker for plugin from file {}: {}",
                        file, e
                    );
                }
            }
        }
    });

    // Spawn a task to clean up completed jobs periodically
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5 * 60)).await; // Every 5 minutes

            let mut jobs_lock = jobs.write().await;
            let before_len = jobs_lock.len();

            // Filter out jobs that completed more than 60 minutes ago
            jobs_lock.retain(|job| {
                if let Some(end_time) = job.end_time {
                    let elapsed = end_time.elapsed();
                    elapsed.as_secs() < 60 * 60 // Keep if less than 60 minutes old
                } else {
                    true // Keep all jobs that haven't completed yet
                }
            });

            let removed = before_len - jobs_lock.len();
            if removed > 0 {
                tracing::info!(
                    "Cleaned up {} completed jobs older than 60 minutes",
                    removed
                );
            }
        }
    });

    tracing::info!("Binding to address: {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Starting to serve...");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injector::Metadata;
    use crate::routes::UploadPluginResponse;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::header;
    use axum::http::Request;
    use axum::http::StatusCode;
    use axum::response::Response;
    use serde_json::json;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn test_plugins_list_empty() {
        let app_state = AppState::default();

        let app = Router::new()
            .route("/plugins/list", get(plugins_list))
            .with_state(app_state);

        let response = app
            .oneshot(
                Request::get("/plugins/list")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let r = response.into_body();
        let body_bytes = to_bytes(r, usize::MAX).await.unwrap();
        let body: Vec<Metadata> = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn test_pipelines_list_empty() {
        let app_state = AppState::default();

        let app = Router::new()
            .route("/pipelines/list", get(pipelines_list))
            .with_state(app_state);

        let response = app
            .oneshot(
                Request::get("/pipelines/list")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let r = response.into_body();
        let body_bytes = to_bytes(r, usize::MAX).await.unwrap();
        let body: Vec<(String, Pipeline)> = serde_json::from_slice(&body_bytes).unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn test_plugin_execute_not_found() {
        let app_state = AppState::default();

        let app = Router::new()
            .route("/plugins/{id}/execute", post(plugin_execute))
            .with_state(app_state);

        let body: Vec<u8> = serde_json::to_vec(&json!("{}")).unwrap();
        let response = app
            .oneshot(
                Request::post("/plugins/nonexistent/execute")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_execute_pipeline_not_found() {
        let app_state = AppState::default();

        let app = Router::new()
            .route("/pipelines/{id}/execute", post(pipelines_execute))
            .with_state(app_state);

        let response = app
            .oneshot(
                Request::post("/pipelines/nonexistent/execute")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_vec(&json!("{}")).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_register_plugin_from_wasm() {
        let (app_state, _, response) = setup_adder_server().await;

        let mut failed = false;
        if response.status() != StatusCode::CREATED {
            failed = true;
        }

        // Verify the plugin was registered
        let r = response.into_body();
        let body_bytes = to_bytes(r, usize::MAX).await.unwrap();
        if failed {
            let response = String::from_utf8_lossy(&body_bytes);
            println!("Error message: {}", response);
            panic!("Failed to upload plugin");
        }
        let response: UploadPluginResponse = serde_json::from_slice(&body_bytes).unwrap();

        assert!(response.success);

        // Verify the plugin exists in the plugin manager
        let plugins = app_state.plugin_manager.read().await;
        assert!(plugins.contains_key(&response.id));
    }

    async fn setup_adder_server() -> (AppState, Router, Response<Body>) {
        use std::env;
        use std::path::PathBuf;

        // Get the CARGO_TARGET_DIR or use a default
        let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target");

        // Construct path to test WASM file
        let wasm_path = target_dir
            .join("wasm32-wasip2")
            .join("debug")
            .join("simple_adder.wasm");

        // Skip the test if the file doesn't exist
        if !wasm_path.exists() {
            eprintln!("Test WASM file not found at {:?}, skipping test", wasm_path);
            panic!("here");
        }

        // Read the WASM file
        let wasm_bytes = std::fs::read(&wasm_path).expect("Failed to read WASM file");

        let app_state = AppState::default();

        let app = Router::new()
            .route("/plugins/upload", post(plugin_upload))
            .route("/plugins/{id}/execute", post(pipelines_execute))
            .route("/pipelines/register", post(pipelines_register))
            .route("/pipelines/{id}/execute", post(pipelines_execute))
            .layer(DefaultBodyLimit::disable())
            .with_state(app_state.clone());

        // Test uploading the plugin
        let response = app
            .clone()
            .oneshot(
                Request::post("/plugins/upload")
                    // .header(header::CONTENT_TYPE, "application/octet-stream")
                    .body(axum::body::Body::from(wasm_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        (app_state, app, response)
    }
}
