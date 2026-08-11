use crate::daemon::Daemon;
use crate::daemon::SyncDaemonRegistry;
use crate::pipeline::Pipeline;
use crate::pipeline::SyncPipelineRegistry;
use crate::pipeline::SyncUserToPipelineRef;
use crate::plugin::SyncPluginRegistry;
use crate::plugin::SyncUserToPluginRef;
use crate::{
    daemon::SyncIdToDaemonRef,
    plugin::StringToStringWorker,
    tenant::{auth::jwt::JwtManager, org::Organization, user::User},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub mod auth;
pub mod org;
pub mod user;

pub type ApiKeyRegistry = HashMap<String, String>;
pub type SyncApiKeyRegistry = Arc<RwLock<ApiKeyRegistry>>;
pub type OrganizationRegistry = HashMap<String, Organization>;
pub type SyncOrganizationRegistry = Arc<RwLock<OrganizationRegistry>>;
pub type UserRegistry = HashMap<String, User>;
pub type SyncUserRegistry = Arc<RwLock<UserRegistry>>;

#[derive(Clone, Debug, Default)]
pub struct TenantState {
    /// Whether authentication is enforced for the tenant
    pub enforce_auth: bool,

    /// Central registry of all plugins (hash -> plugin)
    pub plugin_registry: SyncPluginRegistry,

    /// Central registry of all pipelines (hash -> pipeline)
    pub pipeline_registry: SyncPipelineRegistry,

    /// Central registry of all daemons (hash -> daemon)
    pub daemon_registry: SyncDaemonRegistry,

    /// Organization plugin references (org_id -> set of plugin hashes)
    pub org_plugin_refs: SyncUserToPluginRef,
    /// Organization daemons (org_id -> daemon_id -> daemon)
    pub org_daemons: SyncIdToDaemonRef,
    /// Organization pipelines (org_id -> pipeline_id -> pipeline)
    pub org_pipelines: SyncUserToPipelineRef,

    /// User plugin references (user_id -> set of plugin hashes) - plugins created by the user
    pub user_plugin_refs: SyncUserToPluginRef,
    /// User visible plugins (user_id -> set of plugin hashes) - all plugins user can access
    pub user_visible_plugins: SyncUserToPluginRef,

    /// User-specific pipelines (user_id -> pipeline_id -> pipeline)
    pub user_pipeline_refs: SyncUserToPipelineRef,
    /// User-specific daemons (user_id -> daemon_id -> daemon)
    pub user_daemons: SyncIdToDaemonRef,

    /// Organizations in the system
    pub organizations: SyncOrganizationRegistry,
    /// Users in the system
    pub users: SyncUserRegistry,
    /// API key to user mapping (api_key -> user_id)
    pub api_key_to_user: SyncApiKeyRegistry,
    /// JWT manager for token handling
    pub jwt_manager: JwtManager,
}

impl TenantState {
    /// Create a new TenantState instance
    pub fn new(secret_key: Option<String>) -> Self {
        Self {
            enforce_auth: secret_key.is_some(),
            plugin_registry: Arc::new(RwLock::new(HashMap::new())),
            pipeline_registry: Arc::new(RwLock::new(HashMap::new())),
            daemon_registry: Arc::new(RwLock::new(HashMap::new())),
            org_plugin_refs: Arc::new(RwLock::new(HashMap::new())),
            org_daemons: Arc::new(RwLock::new(HashMap::new())),
            org_pipelines: Arc::new(RwLock::new(HashMap::new())),
            user_plugin_refs: Arc::new(RwLock::new(HashMap::new())),
            user_visible_plugins: Arc::new(RwLock::new(HashMap::new())),
            user_pipeline_refs: Arc::new(RwLock::new(HashMap::new())),
            user_daemons: Arc::new(RwLock::new(HashMap::new())),
            organizations: Arc::new(RwLock::new(HashMap::new())),
            users: Arc::new(RwLock::new(HashMap::new())),
            api_key_to_user: Arc::new(RwLock::new(HashMap::new())),
            jwt_manager: JwtManager::new(secret_key.unwrap_or_default()),
        }
    }

    pub fn hash_wasm_bytes(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Register a plugin in the central registry and return its hash
    pub async fn register_plugin(
        &self,
        wasm_bytes: &[u8],
    ) -> Result<(String, StringToStringWorker), String> {
        let plugin_hash = Self::hash_wasm_bytes(wasm_bytes);

        // Check if the plugin already exists in the registry
        {
            let registry = self.plugin_registry.read().await;
            if let Some(worker) = registry.get(&plugin_hash) {
                return Ok((plugin_hash, worker.clone()));
            }
        }

        // Create a new worker
        let worker = match StringToStringWorker::from_bytes(
            self.plugin_registry.clone(), // This will be filled once the plugin is registered
            self.user_visible_plugins.clone(),
            self.organizations.clone(),
            wasm_bytes,
        )
        .await
        {
            Ok(worker) => worker,
            Err(e) => return Err(e.to_string()),
        };

        // Store the worker in the registry
        {
            let mut registry = self.plugin_registry.write().await;
            registry.insert(plugin_hash.clone(), worker.clone());
        }

        Ok((plugin_hash, worker))
    }

    /// Get a plugin by its hash
    pub async fn get_plugin(&self, plugin_hash: &str) -> Option<StringToStringWorker> {
        self.plugin_registry.read().await.get(plugin_hash).cloned()
    }

    /// Add a plugin to a user's created plugins
    pub async fn add_user_plugin(
        &self,
        username: &str,
        plugin_id: &str,
        plugin_hash: &str,
    ) -> Result<(), String> {
        // Add to user's created plugins
        let mut to_remove = None;
        {
            let mut user_plugins = self.user_plugin_refs.write().await;
            let user_plugin_set = user_plugins
                .entry(username.to_string())
                .or_insert_with(HashSet::new);

            // Check the plugin registry to make sure these plugins exist
            let plugin_registry = self.plugin_registry.read().await;
            for user_plugin_hash in user_plugin_set.iter() {
                if let Some(plugin) = plugin_registry.get(user_plugin_hash) {
                    if plugin.plugin_id.as_ref().unwrap() == plugin_id {
                        to_remove = Some(user_plugin_hash.clone());
                        // Update the hash in the corresponding organization's plugins
                        let users = self.users.read().await;
                        if let Some(user) = users.get(username) {
                            for org_id in &user.organization_ids {
                                // update the plugin in the org's plugins
                                let mut org_plugins = self.org_plugin_refs.write().await;
                                if let Some(org_plugin_set) = org_plugins.get_mut(org_id) {
                                    if org_plugin_set.remove(user_plugin_hash) {
                                        org_plugin_set.insert(plugin_hash.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(ref to_remove) = to_remove {
                user_plugin_set.remove(to_remove);
            }

            user_plugin_set.insert(plugin_hash.to_string());
        }

        // Also make it visible to the user
        {
            let mut visible_plugins = self.user_visible_plugins.write().await;
            let visible_set = visible_plugins
                .entry(username.to_string())
                .or_insert_with(HashSet::new);
            if let Some(to_remove) = to_remove {
                visible_set.remove(&to_remove);
            }
            visible_set.insert(plugin_hash.to_string());
        }

        if !self.is_plugin_in_use(plugin_hash).await {
            self.plugin_registry.write().await.remove(plugin_hash);
        }

        Ok(())
    }

    /// Share a plugin with an organization
    pub async fn share_plugin_with_org(
        &self,
        plugin_hash: &str,
        org_id: &str,
    ) -> Result<(), String> {
        // Add to organization's plugins
        {
            let mut org_plugins = self.org_plugin_refs.write().await;
            let org_plugin_set = org_plugins
                .entry(org_id.to_string())
                .or_insert_with(HashSet::new);
            org_plugin_set.insert(plugin_hash.to_string());
        }

        // Add to all organization members' visible plugins
        let members = {
            let orgs = self.organizations.read().await;
            if let Some(org) = orgs.get(org_id) {
                org.member_ids.clone()
            } else {
                return Err(format!("Organization {} not found", org_id));
            }
        };

        for member_id in members {
            let mut visible_plugins = self.user_visible_plugins.write().await;
            let visible_set = visible_plugins
                .entry(member_id.to_string())
                .or_insert_with(HashSet::new);
            visible_set.insert(plugin_hash.to_string());
        }

        Ok(())
    }

    pub async fn user_plugin_id_to_hash(&self, username: &str, plugin_id: &str) -> Option<String> {
        let plugin_registry = self.plugin_registry.read().await;
        let user_plugins = self.user_plugin_refs.read().await;

        if let Some(plugin_hashes) = user_plugins.get(username) {
            for hash in plugin_hashes {
                if let Some(plugin) = plugin_registry.get(hash) {
                    if let Some(ref id) = plugin.plugin_id {
                        if id == plugin_id {
                            return Some(hash.clone());
                        }
                    }
                }
            }
        }

        None
    }

    /// Get all plugins visible to a user
    pub async fn get_user_visible_plugins(
        &self,
        username: &str,
    ) -> HashMap<String, StringToStringWorker> {
        let mut result = HashMap::new();
        let registry = self.plugin_registry.read().await;

        if let Some(plugin_hashes) = self.user_visible_plugins.read().await.get(username) {
            for hash in plugin_hashes {
                if let Some(plugin) = registry.get(hash) {
                    result.insert(hash.clone(), plugin.clone());
                }
            }
        }

        result
    }

    /// Get plugins created by a user
    pub async fn get_user_created_plugins(
        &self,
        username: &str,
    ) -> HashMap<String, StringToStringWorker> {
        let mut result = HashMap::new();
        let registry = self.plugin_registry.read().await;

        if let Some(plugin_hashes) = self.user_plugin_refs.read().await.get(username) {
            for hash in plugin_hashes {
                if let Some(plugin) = registry.get(hash) {
                    result.insert(hash.clone(), plugin.clone());
                }
            }
        }

        result
    }

    /// Get plugins for an organization
    pub async fn get_org_plugins(&self, org_id: &str) -> HashMap<String, StringToStringWorker> {
        let mut result = HashMap::new();
        let registry = self.plugin_registry.read().await;

        if let Some(plugin_hashes) = self.org_plugin_refs.read().await.get(org_id) {
            for hash in plugin_hashes {
                if let Some(plugin) = registry.get(hash) {
                    result.insert(hash.clone(), plugin.clone());
                }
            }
        }

        result
    }

    /// Update a plugin's references when a user is added to an organization
    pub async fn update_user_plugin_visibility(
        &self,
        username: &str,
        org_id: &str,
    ) -> Result<(), String> {
        let org_plugins = {
            let org_refs = self.org_plugin_refs.read().await;
            if let Some(plugins) = org_refs.get(org_id) {
                plugins.clone()
            } else {
                HashSet::new()
            }
        };

        if !org_plugins.is_empty() {
            let mut visible_plugins = self.user_visible_plugins.write().await;
            let visible_set = visible_plugins
                .entry(username.to_string())
                .or_insert_with(HashSet::new);

            for plugin_hash in org_plugins {
                visible_set.insert(plugin_hash);
            }
        }

        Ok(())
    }

    /// Remove a plugin from a user's created plugins
    pub async fn remove_user_plugin(
        &self,
        username: &str,
        plugin_hash: &str,
    ) -> Result<(), String> {
        // Remove from user's created plugins
        {
            let mut user_plugins = self.user_plugin_refs.write().await;
            if let Some(plugin_set) = user_plugins.get_mut(username) {
                plugin_set.remove(plugin_hash);
            }
        }

        // Maybe remove from user's visible plugins if not shared via organization
        let should_remove = {
            let orgs = self.organizations.read().await;
            let users = self.users.read().await;
            let user = users.get(username);

            let mut is_shared_via_org = false;
            if let Some(user) = user {
                for org_id in &user.organization_ids {
                    if let Some(_org) = orgs.get(org_id) {
                        let org_plugins = self.org_plugin_refs.read().await;
                        if let Some(plugins) = org_plugins.get(org_id) {
                            if plugins.contains(plugin_hash) {
                                is_shared_via_org = true;
                                break;
                            }
                        }
                    }
                }
            }
            !is_shared_via_org
        };

        if should_remove {
            let mut visible_plugins = self.user_visible_plugins.write().await;
            if let Some(visible_set) = visible_plugins.get_mut(username) {
                visible_set.remove(plugin_hash);
            }
        }

        Ok(())
    }

    /// Check if any user or org is using this plugin
    pub async fn is_plugin_in_use(&self, plugin_hash: &str) -> bool {
        // Check user references
        {
            let user_refs = self.user_plugin_refs.read().await;
            for (_, plugins) in user_refs.iter() {
                if plugins.contains(plugin_hash) {
                    return true;
                }
            }
        }

        // Check org references
        {
            let org_refs = self.org_plugin_refs.read().await;
            for (_, plugins) in org_refs.iter() {
                if plugins.contains(plugin_hash) {
                    return true;
                }
            }
        }

        false
    }

    /// Clean up plugins that are no longer referenced
    pub async fn cleanup_unused_plugins(&self) -> usize {
        let mut removed_count = 0;
        let all_plugins = self
            .plugin_registry
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        for plugin_hash in all_plugins {
            if !self.is_plugin_in_use(&plugin_hash).await {
                self.plugin_registry.write().await.remove(&plugin_hash);
                removed_count += 1;
            }
        }

        removed_count
    }

    pub async fn register_daemon(&self, daemon_id: String, daemon: Daemon) -> Result<(), String> {
        // Add daemon to central registry
        self.daemon_registry
            .write()
            .await
            .insert(daemon_id.clone(), daemon);
        Ok(())
    }

    pub async fn get_daemon(&self, daemon_id: &str) -> Option<Daemon> {
        self.daemon_registry.read().await.get(daemon_id).cloned()
    }

    pub async fn add_daemon_to_user(&self, username: &str, daemon_id: &str) -> Result<(), String> {
        let mut user_daemons = self.user_daemons.write().await;
        let daemon_set = user_daemons
            .entry(username.to_string())
            .or_insert_with(HashSet::new);
        daemon_set.insert(daemon_id.to_string());
        Ok(())
    }

    pub async fn add_daemon_to_org(&self, org_id: &str, daemon_id: &str) -> Result<(), String> {
        let mut org_daemons = self.org_daemons.write().await;
        let daemon_set = org_daemons
            .entry(org_id.to_string())
            .or_insert_with(HashSet::new);
        daemon_set.insert(daemon_id.to_string());
        Ok(())
    }

    // Similar methods for pipelines

    pub async fn register_pipeline(
        &self,
        pipeline_id: String,
        pipeline: Pipeline,
    ) -> Result<(), String> {
        self.pipeline_registry
            .write()
            .await
            .insert(pipeline_id.clone(), pipeline);
        Ok(())
    }

    pub async fn get_pipeline(&self, pipeline_id: &str) -> Option<Pipeline> {
        self.pipeline_registry
            .read()
            .await
            .get(pipeline_id)
            .cloned()
    }

    pub async fn add_pipeline_to_user(
        &self,
        username: &str,
        pipeline_id: &str,
    ) -> Result<(), String> {
        let mut user_pipelines = self.user_pipeline_refs.write().await;
        let pipeline_set = user_pipelines
            .entry(username.to_string())
            .or_insert_with(HashSet::new);
        pipeline_set.insert(pipeline_id.to_string());
        Ok(())
    }

    pub async fn add_pipeline_to_org(&self, org_id: &str, pipeline_id: &str) -> Result<(), String> {
        let mut org_pipelines = self.org_pipelines.write().await;
        let pipeline_set = org_pipelines
            .entry(org_id.to_string())
            .or_insert_with(HashSet::new);
        pipeline_set.insert(pipeline_id.to_string());
        Ok(())
    }

    // Methods to retrieve sets of daemons/pipelines by user or org

    pub async fn get_user_daemons(&self, username: &str) -> HashMap<String, Daemon> {
        let mut result = HashMap::new();
        let registry = self.daemon_registry.read().await;

        if let Some(daemon_ids) = self.user_daemons.read().await.get(username) {
            for id in daemon_ids {
                if let Some(daemon) = registry.get(id) {
                    result.insert(id.clone(), daemon.clone());
                }
            }
        }

        result
    }

    pub async fn get_org_daemons(&self, org_id: &str) -> HashMap<String, Daemon> {
        let mut result = HashMap::new();
        let registry = self.daemon_registry.read().await;

        if let Some(daemon_ids) = self.org_daemons.read().await.get(org_id) {
            for id in daemon_ids {
                if let Some(daemon) = registry.get(id) {
                    result.insert(id.clone(), daemon.clone());
                }
            }
        }

        result
    }

    // Similar methods for pipelines

    pub async fn get_user_pipelines(&self, username: &str) -> HashMap<String, Pipeline> {
        let mut result = HashMap::new();
        let registry = self.pipeline_registry.read().await;

        if let Some(pipeline_ids) = self.user_pipeline_refs.read().await.get(username) {
            for id in pipeline_ids {
                if let Some(pipeline) = registry.get(id) {
                    result.insert(id.clone(), pipeline.clone());
                }
            }
        }

        result
    }

    pub async fn get_org_pipelines(&self, org_id: &str) -> HashMap<String, Pipeline> {
        let mut result = HashMap::new();
        let registry = self.pipeline_registry.read().await;

        if let Some(pipeline_ids) = self.org_pipelines.read().await.get(org_id) {
            for id in pipeline_ids {
                if let Some(pipeline) = registry.get(id) {
                    result.insert(id.clone(), pipeline.clone());
                }
            }
        }

        result
    }
}

/// User roles within an organization
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Can view resources but not modify
    Viewer,
    /// Can create and execute but not manage
    Contributor,
    /// Can manage resources but not users
    Manager,
    /// Can manage users and all resources
    Admin,
    /// Owner of the organization
    Owner,
}

/// Types of resources in the system
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Plugin,
    Pipeline,
    Job,
    Daemon,
    User,
    Organization,
    VectorStore,
}

/// Actions that can be performed on resources
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Action {
    View,
    Create,
    Edit,
    Delete,
    Execute,
    Share,
    Manage,
}
