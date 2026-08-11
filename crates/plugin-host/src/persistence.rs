use crate::daemon::Daemon;
use crate::pipeline::Pipeline;
use crate::plugin::new_worker;
use crate::server::AppState;
use crate::tenant::org::Organization;
use crate::tenant::user::User;
use axum::extract::Request;
use axum::extract::State;
use axum::response::Response;
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use tokio::task;

/// Represents the serializable state for persistence
#[derive(Serialize, Deserialize)]
pub struct PersistentState {
    /// Serialized users
    pub users: HashMap<String, User>,
    /// Serialized organizations
    pub organizations: HashMap<String, Organization>,
    /// API key to user mapping
    pub api_key_to_user: HashMap<String, String>,
    /// Serialized user plugins (plugin hashes that each user created)
    pub user_plugin_refs: HashMap<String, HashSet<String>>,
    /// Plugin registry (hash -> base64 encoded binary)
    pub plugin_registry: HashMap<String, String>,
    /// Organization plugin references
    pub org_plugin_refs: HashMap<String, HashSet<String>>,
    /// User pipeline references
    pub user_pipeline_refs: HashMap<String, HashSet<String>>,
    /// Pipeline registry (id -> pipeline)
    pub pipeline_registry: HashMap<String, Pipeline>,
    /// Organization pipeline references
    pub org_pipelines: HashMap<String, HashSet<String>>,
    /// User daemon references
    pub user_daemons: HashMap<String, HashSet<String>>,
    /// Daemon registry (id -> daemon)
    pub daemon_registry: HashMap<String, Daemon>,
    /// Organization daemon references
    pub org_daemons: HashMap<String, HashSet<String>>,
}

impl PersistentState {
    pub async fn from(state: &AppState) -> Self {
        // We'll need to acquire locks to copy the data
        // But this happens in an async context so we can't block
        let users = state.tenant_state.users.read().await.clone();
        let organizations = state.tenant_state.organizations.read().await.clone();
        let api_key_to_user = state.tenant_state.api_key_to_user.read().await.clone();

        // Extract user plugin references
        let user_plugin_refs = state.tenant_state.user_plugin_refs.read().await.clone();

        // Extract organization plugin references
        let org_plugin_refs = state.tenant_state.org_plugin_refs.read().await.clone();

        // Extract the central plugin registry
        let plugin_registry = state
            .tenant_state
            .plugin_registry
            .read()
            .await
            .iter()
            .map(|(hash, worker)| {
                (
                    hash.clone(),
                    BASE64_STANDARD.encode(worker.raw_binary.clone()),
                )
            })
            .collect();

        // Extract user pipeline references
        let user_pipeline_refs = state.tenant_state.user_pipeline_refs.read().await.clone();

        // Extract pipeline registry
        let pipeline_registry = state.tenant_state.pipeline_registry.read().await.clone();

        // Extract organization pipeline references
        let org_pipelines = state.tenant_state.org_pipelines.read().await.clone();

        // Extract user daemon references
        let user_daemons = state.tenant_state.user_daemons.read().await.clone();

        // Extract daemon registry
        let daemon_registry = state.tenant_state.daemon_registry.read().await.clone();

        // Extract organization daemon references
        let org_daemons = state.tenant_state.org_daemons.read().await.clone();

        Self {
            users,
            organizations,
            api_key_to_user,
            user_plugin_refs,
            plugin_registry,
            org_plugin_refs,
            user_pipeline_refs,
            pipeline_registry,
            org_pipelines,
            user_daemons,
            daemon_registry,
            org_daemons,
        }
    }
}

/// Persistence service for saving and loading application state
#[derive(Debug)]
pub struct PersistenceService {
    /// Directory where state files are stored
    pub storage_dir: PathBuf,
    /// Filename for the state file
    pub filename: String,
}

impl PersistenceService {
    /// Create a new persistence service
    pub fn new(storage_dir: impl Into<PathBuf>, filename: impl Into<String>) -> Self {
        let storage_dir = storage_dir.into();
        fs::create_dir_all(&storage_dir).expect("Failed to create storage directory");

        Self {
            storage_dir,
            filename: filename.into(),
        }
    }

    /// Get the full path to the state file
    pub fn state_path(&self) -> PathBuf {
        self.storage_dir.join(&self.filename)
    }

    /// Load state from disk
    pub fn load(&self) -> Option<PersistentState> {
        let path = self.state_path();
        if !path.exists() {
            return None;
        }

        match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(state) => Some(state),
                Err(e) => {
                    tracing::error!("Failed to parse state file: {}", e);
                    None
                }
            },
            Err(e) => {
                tracing::error!("Failed to read state file: {}", e);
                None
            }
        }
    }

    /// Save state to disk
    pub fn save(&self, state: &PersistentState) -> Result<(), String> {
        let json = match serde_json::to_string_pretty(state) {
            Ok(json) => json,
            Err(e) => return Err(format!("Failed to serialize state: {}", e)),
        };

        let path = self.state_path();

        // Ensure the parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(format!("Failed to create state directory: {}", e));
                }
            }
        }

        let temp_path = path.with_extension("tmp");

        // First write to a temporary file
        if let Err(e) = fs::write(&temp_path, json) {
            return Err(format!("Failed to write temporary state file: {}", e));
        }

        // Then atomically rename it to the final path
        if let Err(e) = fs::rename(&temp_path, &path) {
            return Err(format!("Failed to rename state file: {}", e));
        }

        Ok(())
    }

    /// Save state asynchronously without blocking the current thread
    pub fn save_async(&self, state: AppState) {
        let service = self.clone();

        task::spawn(async move {
            let persistent_state = PersistentState::from(&state).await;
            if let Err(e) = service.save(&persistent_state) {
                tracing::error!("Failed to save state: {}", e);
            } else {
                tracing::debug!("State saved successfully");
            }
        });
    }
}

impl Clone for PersistenceService {
    fn clone(&self) -> Self {
        Self {
            storage_dir: self.storage_dir.clone(),
            filename: self.filename.clone(),
        }
    }
}

pub async fn persistence_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    // Process the request first
    let response = next.run(req).await;

    // Save state after processing the request
    // This happens in the background and won't block the response
    state.persistence.save_async(state.clone());

    response
}

/// Attempt to restore state from persistence
pub async fn load_state(service: &PersistenceService, app_state: AppState) -> bool {
    if let Some(persistent_state) = service.load() {
        // Update the app state with the loaded data
        *app_state.tenant_state.users.write().await = persistent_state.users;
        *app_state.tenant_state.organizations.write().await = persistent_state.organizations;
        *app_state.tenant_state.api_key_to_user.write().await = persistent_state.api_key_to_user;

        // Restore user pipeline references
        *app_state.tenant_state.user_pipeline_refs.write().await =
            persistent_state.user_pipeline_refs;

        // Restore pipeline registry
        *app_state.tenant_state.pipeline_registry.write().await =
            persistent_state.pipeline_registry;

        // Restore organization pipeline references
        *app_state.tenant_state.org_pipelines.write().await = persistent_state.org_pipelines;

        // Restore user daemon references
        *app_state.tenant_state.user_daemons.write().await = persistent_state.user_daemons;

        // Restore daemon registry
        *app_state.tenant_state.daemon_registry.write().await = persistent_state.daemon_registry;

        // Restore organization daemon references
        *app_state.tenant_state.org_daemons.write().await = persistent_state.org_daemons;

        // Process plugins
        let mut plugin_registry = app_state.tenant_state.plugin_registry.write().await;
        for (plugin_hash, bin_base64) in persistent_state.plugin_registry {
            let decoded_bin = match BASE64_STANDARD.decode(&bin_base64) {
                Ok(bin) => bin,
                Err(e) => {
                    tracing::error!("Failed to decode plugin binary: {}", e);
                    continue;
                }
            };

            match new_worker(
                &decoded_bin,
                app_state.tenant_state.plugin_registry.clone(),
                app_state.tenant_state.user_visible_plugins.clone(),
                app_state.tenant_state.organizations.clone(),
            )
            .await
            {
                Ok((_, worker)) => {
                    plugin_registry.insert(plugin_hash, worker);
                }
                Err(e) => {
                    tracing::error!("Failed to load plugin: {}", e);
                }
            }
        }
        drop(plugin_registry);

        // Restore user plugin references
        *app_state.tenant_state.user_plugin_refs.write().await = persistent_state.user_plugin_refs;

        // Restore organization plugin references
        *app_state.tenant_state.org_plugin_refs.write().await = persistent_state.org_plugin_refs;

        // Update user visible plugins based on references
        let users = app_state.tenant_state.users.read().await;
        let user_plugin_refs = app_state.tenant_state.user_plugin_refs.read().await;
        let org_plugin_refs = app_state.tenant_state.org_plugin_refs.read().await;

        let mut user_visible_plugins = app_state.tenant_state.user_visible_plugins.write().await;

        // Initialize visible plugins for each user
        for (username, _) in users.iter() {
            let mut visible_plugins = HashSet::new();

            // Add user's own plugins
            if let Some(plugins) = user_plugin_refs.get(username) {
                visible_plugins.extend(plugins.clone());
            }

            // Add plugins from all organizations the user belongs to
            if let Some(user) = users.get(username) {
                for org_id in &user.organization_ids {
                    if let Some(org_plugins) = org_plugin_refs.get(org_id) {
                        visible_plugins.extend(org_plugins.clone());
                    }
                }
            }

            user_visible_plugins.insert(username.clone(), visible_plugins);
        }

        tracing::info!("Restored state from persistence");
        true
    } else {
        tracing::info!("No persistent state found, starting with empty state");
        false
    }
}
