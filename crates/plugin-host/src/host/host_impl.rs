use crate::injector::exports::plugin::injector::guest::PluginKind;
use crate::injector::host::MetaToolInfo;
use crate::injector::host::MetaTools;
use crate::injector::ToolSchema;
use crate::injector::{
    env, error,
    error::FsError,
    error::PluginError,
    host, http, logger, markdown_converter, open_a_i_like,
    open_a_i_like::{ChatCompletion, ChatConfig, EmbeddingInput, EncodingFormat, ToolSelection},
    vector_db,
    vector_db::{SimilarityResponse, SimilaritySearchConfig},
};
use crate::injector::{metadata_to_meta_tool, metadata_to_tool_schema};
use crate::plugin::SyncPluginRegistry;
use crate::plugin::SyncUserToPluginRef;
use crate::routes::PipelineJob;
use crate::server::SseStreamTx;
use crate::tenant::user::fs::user_path;
use crate::tenant::user::User;
use crate::tenant::SyncOrganizationRegistry;
/// This module provides the host implementation for Concordance plugins.
///
/// It includes functionality for resource management, HTTP operations,
/// file conversions, vector database operations, and LLM interactions.
use crate::{
    host::{
        llm::{chat_create, embeddings_create},
        normalize_relative_path, writeable_path,
    },
    tenant::org::SharedEnvVar,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use wasmtime::component::Resource;
use wasmtime::component::ResourceTable;

use super::providers::{
    convert::MdConverter,
    http::ReqwestHttp,
    llm::{ChatSession, Client, OpenAIConfig},
    vectordb::DbConn,
};

/// Host implementation for Concordance plugins.
///
/// The Host manages plugin resources, provides API interfaces, and
/// handles communication between plugins and external services.
pub struct Host {
    /// If the user requests SSE stream, this field will contain a sender for SSE events.
    pub sse_stream_tx: Option<SseStreamTx>,
    /// The current plugin id
    pub plugin_id: String,
    /// Resource table for managing plugin resources
    pub resources: ResourceTable,
    /// HTTP client for making web requests
    http: ReqwestHttp,
    /// Markdown converter for file transformations
    md: MdConverter,
    /// Registry of available plugins that can be called
    pub plugin_registry: SyncPluginRegistry,
    /// Registry of available plugins that are visible to the user
    pub visible_plugin_hashes: SyncUserToPluginRef,
    /// Registry of available organizations
    pub orgs: SyncOrganizationRegistry,
    /// Current execution status
    pub status: Option<String>,
    /// Current job identifier if part of a pipeline
    pub job_id: Option<String>,
    /// List of all jobs in the pipeline
    pub jobs: Option<Arc<RwLock<Vec<PipelineJob>>>>,
    /// User information
    pub user: Option<User>,
}

impl Host {
    /// Creates a new Host instance.
    ///
    /// # Arguments
    ///
    /// * `other_plugins` - Available plugins that can be called by this host
    /// * `jobs` - Optional list of pipeline jobs
    /// * `job_id` - Optional ID of the current job
    /// * `organization` - Optional organization information
    /// * `user` - Optional user information
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plugin_id: String,
        plugin_registry: SyncPluginRegistry,
        visible_plugin_hashes: SyncUserToPluginRef,
        orgs: SyncOrganizationRegistry,
        jobs: Option<Arc<RwLock<Vec<PipelineJob>>>>,
        job_id: Option<String>,
        user: Option<User>,
        sse_stream_tx: Option<SseStreamTx>,
    ) -> Self {
        tracing::debug!("new host - streaming: {}", sse_stream_tx.is_some());
        Self {
            plugin_id,
            resources: ResourceTable::new(),
            http: ReqwestHttp::new(),
            md: MdConverter::new(),
            plugin_registry,
            visible_plugin_hashes,
            orgs,
            status: None,
            job_id,
            jobs,
            user,
            sse_stream_tx,
        }
    }

    /// Returns a mutable reference to the HTTP provider.
    pub fn http(&mut self) -> &mut ReqwestHttp {
        &mut self.http
    }

    /// Returns a mutable reference to the Markdown converter.
    pub fn md(&mut self) -> &mut MdConverter {
        &mut self.md
    }
}

pub async fn user_plugin_hash_by_name(
    user: Option<&User>,
    visible_plugin_hashes: SyncUserToPluginRef,
    plugin_registry: SyncPluginRegistry,
    name: &String,
) -> Option<String> {
    let user = user.as_ref()?;
    let available_hashes = visible_plugin_hashes.read().await;
    let hashes = available_hashes.get(&user.username)?;
    let all_plugins = plugin_registry.read().await;
    hashes
        .iter()
        .find(|hash| {
            all_plugins
                .get(*hash)
                .map(|plugin| plugin.plugin_id.as_ref().unwrap() == name)
                .unwrap_or(false)
        })
        .cloned()
}

// Core Host Implementation
impl host::Host for Host {
    /// Updates the status of the current job.
    ///
    /// This will update the job status in the pipeline if the host
    /// is part of a pipeline execution.
    async fn update_status(&mut self, status: String) {
        // Update the status of the host
        if let Some(ref id) = self.job_id {
            if let Some(ref jobs) = self.jobs {
                if let Some(job) = jobs.write().await.iter_mut().find(|job| job.id == *id) {
                    job.status = Some(status);
                }
            }
        }
    }

    async fn streaming_enabled(&mut self) -> bool {
        self.sse_stream_tx.is_some()
    }

    /// Converts a file path to a sandbox-safe path.
    ///
    /// Ensures that file operations are contained within the allowed
    /// sandbox directory to prevent unauthorized file access.
    async fn to_sandbox_path(&mut self, file_path: String) -> Result<String, PluginError> {
        let new_file_path = writeable_path().join(std::path::Path::new(&file_path));
        let new_file_path = normalize_relative_path(&new_file_path);
        if !new_file_path.starts_with(writeable_path()) {
            println!(
                "Permission denied: {:?} {:?}",
                new_file_path,
                writeable_path()
            );
            return Err(PluginError::Fs(FsError::PermissionDenied));
        }
        Ok(new_file_path.to_string_lossy().into_owned())
    }

    /// Calls another plugin with the provided input.
    ///
    /// # Arguments
    ///
    /// * `plugin_name` - Name of the plugin to call
    /// * `input` - JSON string input to pass to the plugin
    ///
    /// # Returns
    ///
    /// JSON string output from the plugin
    async fn call_plugin(
        &mut self,
        plugin_name: String,
        input: String,
    ) -> Result<String, PluginError> {
        let input: serde_json::Value =
            serde_json::from_str(&input).map_err(|e| PluginError::Json(e.to_string()))?;

        // First check if the plugin is in the visible plugins for the current context
        let plugin_hash = {
            // Check if user has access to this plugin
            if self.user.is_some() {
                user_plugin_hash_by_name(
                    self.user.as_ref(),
                    self.visible_plugin_hashes.clone(),
                    self.plugin_registry.clone(),
                    &plugin_name,
                )
                .await
            } else {
                // if there is no user, auth must be disabled
                Some(plugin_name.clone())
            }
        };

        if plugin_hash.is_none() {
            return Err(PluginError::Unexpected(format!(
                "Plugin {} is not accessible to the current plugin",
                plugin_name
            )));
        }

        // Then get the plugin from the registry
        let plugin = {
            let plugin_registry = self.plugin_registry.read().await;
            let Some(plugin) = plugin_registry.get(&plugin_hash.unwrap()) else {
                return Err(PluginError::Unexpected(format!(
                    "Plugin {} not found in registry",
                    plugin_name
                )));
            };
            plugin.clone()
        };

        let res = plugin
            .work(&input, self.user.as_ref(), self.sse_stream_tx.clone())
            .await
            .map_err(|e| PluginError::PluginCall(e.to_string()))?;
        Ok(serde_json::to_string(&res).unwrap())
    }

    /// Performs an HTTP GET request.
    async fn get(&mut self, request: http::HttpRequest) -> Result<http::HttpResponse, PluginError> {
        self.http.get(request).await
    }

    /// Performs an HTTP POST request.
    async fn post(
        &mut self,
        request: http::HttpRequest,
    ) -> Result<http::HttpResponse, PluginError> {
        self.http.post(request).await
    }

    /// Logs a message with the specified log level.
    async fn log(&mut self, level: logger::Level, message: String) {
        match level {
            logger::Level::Debug => {
                tracing::debug!(message);
            }
            logger::Level::Info => {
                tracing::info!(message);
            }
            logger::Level::Warn => {
                tracing::warn!(message);
            }
            logger::Level::Error => {
                tracing::error!(message);
            }
        }
    }

    /// Converts a document to markdown format.
    async fn convert(
        &mut self,
        file_type: markdown_converter::FileType,
    ) -> Result<String, PluginError> {
        self.md.convert(file_type).await
    }

    /// Connects to a vector database using the provided connection string.
    ///
    /// The path is sanitized to ensure it's within the sandbox.
    async fn connect_db(
        &mut self,
        connection_string: String,
    ) -> Result<Resource<DbConn>, PluginError> {
        // Enforce path is within writeable directory
        let db_path = if let Some(user) = &self.user {
            user_path(None, &user.username).join(std::path::Path::new(&connection_string))
        } else {
            writeable_path().join(std::path::Path::new(&connection_string))
        };

        let db_path = normalize_relative_path(&db_path);
        if !db_path.starts_with(writeable_path()) {
            println!(
                "Permission denied: {:?} {:?}",
                connection_string,
                writeable_path()
            );
            return Err(PluginError::Fs(FsError::PermissionDenied));
        }

        let db_conn = DbConn::new(db_path.to_string_lossy().to_string()).await;

        let resource = self
            .resources
            .push(db_conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        Ok(resource)
    }

    /// Creates a new OpenAI configuration resource with default settings.
    async fn new_open_ai_config(&mut self) -> Result<Resource<OpenAIConfig>, PluginError> {
        let resource = self
            .resources
            .push(OpenAIConfig::default())
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        Ok(resource)
    }

    /// Creates a new OpenAI client using the provided configuration.
    async fn new_open_ai_client_with_config(
        &mut self,
        config: Resource<OpenAIConfig>,
    ) -> Result<Resource<Client>, PluginError> {
        let config: OpenAIConfig = self
            .resources
            .delete(config)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        self.resources
            .push(Client::with_config(config))
            .map_err(|e| PluginError::ResourceError(e.to_string()))
    }

    /// Creates a new LLM client with the specified base URL and API key.
    async fn new_client(
        &mut self,
        base_url: String,
        api_key: String,
    ) -> Result<Resource<Client>, PluginError> {
        let mut underlying = OpenAIConfig::default().0;
        underlying = underlying.with_api_key(api_key);
        underlying = underlying.with_api_base(base_url);
        self.resources
            .push(Client::with_config(OpenAIConfig(underlying)))
            .map_err(|e| PluginError::ResourceError(e.to_string()))
    }

    async fn mcp_tools(&mut self) -> Result<String, PluginError> {
        let all_plugins = self.plugin_registry.read().await;

        // Determine which plugins the user has access to
        let tools = if let Some(user) = &self.user {
            let username = &user.username;
            let visible_plugin_hashes = self.visible_plugin_hashes.read().await;

            if let Some(accessible_plugin_hashes) = visible_plugin_hashes.get(username) {
                accessible_plugin_hashes
                    .iter()
                    .filter_map(|hash| {
                        all_plugins.get(hash).and_then(|worker| {
                            if worker.metadata.as_ref().unwrap().kind == PluginKind::Tool {
                                worker.metadata.as_ref().map(metadata_to_tool_schema)
                            } else {
                                None
                            }
                        })
                    })
                    .collect::<Vec<ToolSchema>>()
            } else {
                // User exists but has no visible plugins
                Vec::new()
            }
        } else {
            // No user context, include all plugins (auth must be disabled)
            all_plugins
                .iter()
                .filter_map(|(_, worker)| {
                    if worker.metadata.as_ref().unwrap().kind == PluginKind::Tool {
                        Some(metadata_to_tool_schema(worker.metadata.as_ref()?))
                    } else {
                        None
                    }
                })
                .collect::<Vec<ToolSchema>>()
        };

        serde_json::to_string(&tools).map_err(|e| PluginError::Json(e.to_string()))
    }

    async fn mcp_tool_request(&mut self, req: host::ToolsRequest) -> Result<String, PluginError> {
        let all_plugins = self.plugin_registry.read().await;

        // Determine which plugins from the request the user has access to
        let tools = if let Some(user) = &self.user {
            let username = &user.username;
            let visible_plugin_hashes = self.visible_plugin_hashes.read().await;

            if let Some(accessible_plugin_hashes) = visible_plugin_hashes.get(username) {
                let mut all_tools = accessible_plugin_hashes
                    .iter()
                    .filter_map(|hash| {
                        all_plugins.get(hash).and_then(|worker| {
                            worker.metadata.as_ref().map(metadata_to_tool_schema)
                        })
                    })
                    .collect::<Vec<ToolSchema>>();
                // Filter to only include requested plugins that the user has access to
                all_tools.retain(|tool| req.tools_to_include_by_name.contains(&tool.function.name));
                all_tools
            } else {
                // User exists but has no visible plugins
                Vec::new()
            }
        } else {
            // No user context, include all requested plugins (auth must be disabled)
            all_plugins
                .iter()
                .filter_map(|(name, worker)| {
                    if req.tools_to_include_by_name.contains(name) {
                        Some(metadata_to_tool_schema(worker.metadata.as_ref()?))
                    } else {
                        None
                    }
                })
                .collect::<Vec<ToolSchema>>()
        };

        serde_json::to_string(&tools).map_err(|e| PluginError::Json(e.to_string()))
    }

    async fn mcp_meta_tools(&mut self) -> Result<MetaTools, PluginError> {
        tracing::debug!("Fetching meta tools");
        let default_tools_str = env::Host::env_var(self, "DEFAULT_TOOL_IDS".to_string())
            .await
            .unwrap_or_default();
        tracing::debug!("Default tools string: {}", default_tools_str);
        let default_tools: Vec<String> =
            serde_json::from_str(&default_tools_str).unwrap_or_default();
        tracing::debug!("Default tools: {:?}", default_tools);

        let all_plugins = self.plugin_registry.read().await;
        tracing::debug!("Total plugins in registry: {}", all_plugins.len());

        // Determine which plugins the user has access to
        let tools = if let Some(user) = &self.user {
            let username = &user.username;
            tracing::debug!("Getting meta tools for user: {}", username);
            let visible_plugin_hashes = self.visible_plugin_hashes.read().await;

            if let Some(accessible_plugin_hashes) = visible_plugin_hashes.get(username) {
                tracing::debug!(
                    "User has access to {} plugins",
                    accessible_plugin_hashes.len()
                );
                // Filter to include only accessible plugins that aren't in default_tools
                let tools = accessible_plugin_hashes
                    .iter()
                    .filter_map(|hash| {
                        all_plugins.get(hash).and_then(|worker| {
                            if let Some(ref id) = worker.plugin_id {
                                if default_tools.contains(id) {
                                    tracing::debug!("Skipping default tool: {}", id);
                                    return None;
                                }
                            }
                            let tool = metadata_to_meta_tool(worker.metadata.as_ref()?);
                            tracing::debug!("Including tool: {}", tool.name);
                            Some(tool)
                        })
                    })
                    .collect::<Vec<MetaToolInfo>>();
                tracing::info!("Returning {} meta tools for user {}", tools.len(), username);
                tools
            } else {
                tracing::debug!("User {} has no visible plugins", username);
                Vec::new()
            }
        } else {
            tracing::debug!("No user context, returning all non-default tools");
            // No user context, include all plugins except default ones (auth must be disabled)
            let tools = all_plugins
                .iter()
                .filter_map(|(_, worker)| {
                    if let Some(ref id) = worker.plugin_id {
                        if default_tools.contains(id) {
                            tracing::debug!("Skipping default tool: {}", id);
                            return None;
                        }
                    }

                    let tool = metadata_to_meta_tool(worker.metadata.as_ref()?);
                    tracing::debug!("Including tool: {}", tool.name);
                    Some(tool)
                })
                .collect::<Vec<MetaToolInfo>>();
            tracing::info!("Returning {} meta tools (no user context)", tools.len());
            tools
        };

        Ok(MetaTools { tools })
    }
}

impl host::HostSseEvent for Host {
    async fn new(&mut self) -> Resource<axum::response::sse::Event> {
        let event = axum::response::sse::Event::default();
        self.resources.push(event).unwrap()
    }

    async fn set_event(
        &mut self,
        event: Resource<axum::response::sse::Event>,
        name: String,
    ) -> Result<bool, PluginError> {
        let event_obj = self
            .resources
            .get_mut(&event)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        *event_obj = event_obj.clone().event(name);
        Ok(true)
    }

    async fn set_json(
        &mut self,
        event: Resource<axum::response::sse::Event>,
        data: String,
    ) -> Result<bool, PluginError> {
        let event_obj = self
            .resources
            .get_mut(&event)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        *event_obj = event_obj
            .clone()
            .json_data(data)
            .map_err(|e| PluginError::Unexpected(e.to_string()))?;
        Ok(true)
    }

    async fn set_data(
        &mut self,
        event: Resource<axum::response::sse::Event>,
        data: String,
    ) -> Result<bool, PluginError> {
        let event_obj = self
            .resources
            .get_mut(&event)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        *event_obj = event_obj.clone().data(data);
        Ok(true)
    }

    async fn set_id(
        &mut self,
        event: Resource<axum::response::sse::Event>,
        id: String,
    ) -> Result<bool, PluginError> {
        let event_obj = self
            .resources
            .get_mut(&event)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        *event_obj = event_obj.clone().id(id);
        Ok(true)
    }

    async fn send(
        &mut self,
        event: Resource<axum::response::sse::Event>,
    ) -> Result<bool, PluginError> {
        let event = self
            .resources
            .get(&event)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        tracing::debug!("Sending event: {:?}", event);
        if let Some(sse_stream_tx) = self.sse_stream_tx.clone() {
            sse_stream_tx
                .send(Ok(event.clone()))
                .map_err(|e| PluginError::Generic(e.to_string()))?;
        }
        Ok(true)
    }

    async fn drop(&mut self, event: Resource<axum::response::sse::Event>) -> wasmtime::Result<()> {
        let _ = self
            .resources
            .delete(event)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        Ok(())
    }
}

// Implement all the trait implementations for Host
impl error::Host for Host {}
impl http::Host for Host {}
impl markdown_converter::Host for Host {}
impl logger::Host for Host {}
impl vector_db::Host for Host {}
impl open_a_i_like::Host for Host {}

impl env::Host for Host {
    async fn shared_env_var(
        &mut self,
        org_name: String,
        var_name: String,
        plugin_specific: bool,
    ) -> Result<String, PluginError> {
        let orgs = self.orgs.read().await;

        // Find the organization by name
        let org = orgs.get(&org_name).ok_or_else(|| {
            PluginError::Unexpected(format!("Organization '{}' not found", org_name))
        })?;

        if plugin_specific {
            if let Some(SharedEnvVar {
                value: plugin_vars,
                min_role: role,
            }) = org.config.shared_environment.get(&self.plugin_id)
            {
                if let Some(user) = &self.user {
                    if let Some(user_role) = org.member_roles.get(&user.username) {
                        if user_role >= role {
                            let Some(plugin_vars) = plugin_vars.as_object() else {
                                return Err(PluginError::EnvVar(
                                    "Plugin-specific environment variables must be an object"
                                        .to_string(),
                                ));
                            };
                            // Look for the variable in the plugin-specific variables
                            if let Some(value) = plugin_vars.get(&var_name) {
                                // Convert the value to a string
                                match value.as_str() {
                                    Some(str_value) => return Ok(str_value.to_string()),
                                    None => return Ok(value.to_string()),
                                }
                            }
                        }
                        return Err(PluginError::Unexpected(format!(
                            "Insufficient permissions to access environment variable '{}'",
                            var_name
                        )));
                    }
                }
                return Err(PluginError::Unexpected(
                    "No user context available to check permissions".to_string(),
                ));
            }
        } else if let Some(SharedEnvVar {
            value,
            min_role: role,
        }) = org.config.shared_environment.get(&var_name)
        {
            // Check if the user has access to this variable
            if let Some(user) = &self.user {
                if let Some(user_role) = org.member_roles.get(&user.username) {
                    if user_role >= role {
                        return Ok(value.to_string());
                    }
                }
                return Err(PluginError::Unexpected(format!(
                    "Insufficient permissions to access environment variable '{}'",
                    var_name
                )));
            }
            return Err(PluginError::Unexpected(
                "No user context available to check permissions".to_string(),
            ));
        }

        Err(PluginError::Unexpected(format!(
            "Environment variable '{}' not found in organization '{}'",
            var_name, org_name
        )))
    }

    async fn personal_env_var(&mut self, var_name: String) -> Result<String, PluginError> {
        // Check if we have a user context
        if let Some(user) = &self.user {
            // Look for the variable in the user's personal environment variables
            if let Some(value) = user.config.environment_variables.get(&var_name) {
                // Convert the value to a string
                return match value.as_str() {
                    Some(str_value) => Ok(str_value.to_string()),
                    None => Ok(value.to_string()),
                };
            }

            // Variable not found for this user
            return Err(PluginError::Unexpected(format!(
                "Personal environment variable '{}' not found",
                var_name
            )));
        }

        // No user context available
        Err(PluginError::Unexpected(
            "No user context available to access personal environment variables".to_string(),
        ))
    }

    async fn plugin_specific_env_var(&mut self, var_name: String) -> Result<String, PluginError> {
        // Check if we have a user context
        if let Some(user) = &self.user {
            // Try to get the plugin-specific environment variables
            if let Some(plugin_vars) = user.config.environment_variables.get(&self.plugin_id) {
                let Some(plugin_vars) = plugin_vars.as_object() else {
                    return Err(PluginError::EnvVar(
                        "Plugin-specific environment variables must be an object".to_string(),
                    ));
                };
                // Look for the variable in the plugin-specific variables
                if let Some(value) = plugin_vars.get(&var_name) {
                    // Convert the value to a string
                    match value.as_str() {
                        Some(str_value) => return Ok(str_value.to_string()),
                        None => return Ok(value.to_string()),
                    }
                }
            }
        }

        // No user context available
        Err(PluginError::Unexpected(
            "No user context available to access plugin-specific environment variables".to_string(),
        ))
    }

    async fn env_var(&mut self, var_name: String) -> Result<String, PluginError> {
        if let Ok(value) = self.plugin_specific_env_var(var_name.clone()).await {
            return Ok(value);
        }
        // First try to get from personal environment variables
        if let Ok(value) = self.personal_env_var(var_name.clone()).await {
            return Ok(value);
        }

        // If personal env var fails, try getting from organizations
        if let Some(user) = self.user.clone() {
            // Iterate through user's organizations from oldest to newest
            for org_id in &user.organization_ids {
                // Try to get the shared env var from this organization
                if let Ok(value) = self
                    .shared_env_var(org_id.clone(), var_name.clone(), true)
                    .await
                {
                    return Ok(value);
                }
            }

            for org_id in &user.organization_ids {
                // Try to get the shared env var from this organization
                if let Ok(value) = self
                    .shared_env_var(org_id.clone(), var_name.clone(), false)
                    .await
                {
                    return Ok(value);
                }
            }
        }

        // Variable not found in personal or any organization's shared environment
        Err(PluginError::Unexpected(format!(
            "Environment variable '{}' not found",
            var_name
        )))
    }
}

/// Implementation for vector database operations when the vectordb feature is disabled.
///
/// All methods return an error indicating that the feature is not available.
#[cfg(not(feature = "vectordb"))]
impl vector_db::HostDbConnection for Host {
    /// Performs a similarity search (unavailable without vectordb feature).
    async fn similarity_search(
        &mut self,
        _conn: Resource<DbConn>,
        _similarity_config: SimilaritySearchConfig,
        _embedding_client: Resource<Client>,
        _embedding_model: String,
        _table_name: String,
        _input: String,
    ) -> Result<Vec<SimilarityResponse>, PluginError> {
        Err(PluginError::HostDisabled("vectordb feature flag is not enabled. The server owner must recompile with the vectordb feature flag enabled to expose this functionality".to_string()))
    }

    /// Retrieves a row by ID (unavailable without vectordb feature).
    async fn get_row_by_id(
        &mut self,
        _conn: Resource<DbConn>,
        _table_name: String,
        _id_column: String,
        _id_value: String,
        _fields_returned: Vec<String>,
    ) -> Result<Option<String>, PluginError> {
        Err(PluginError::HostDisabled("vectordb feature flag is not enabled. The server owner must recompile with the vectordb feature flag enabled to expose this functionality".to_string()))
    }

    /// Creates a table (unavailable without vectordb feature).
    async fn create_table(
        &mut self,
        _conn: Resource<DbConn>,
        _table_name: String,
        _schema_json: String,
    ) -> Result<bool, PluginError> {
        Err(PluginError::HostDisabled(
            "vectordb feature flag is not enabled. The server owner must recompile with the vectordb feature flag enabled to expose this functionality".to_string(),
        ))
    }

    /// Gets table names (unavailable without vectordb feature).
    async fn get_table_names(
        &mut self,
        _conn: Resource<DbConn>,
    ) -> Result<Vec<String>, PluginError> {
        Err(PluginError::HostDisabled(
            "vectordb feature flag is not enabled. The server owner must recompile with the vectordb feature flag enabled to expose this functionality".to_string(),
        ))
    }

    /// Gets table schema (unavailable without vectordb feature).
    async fn get_table_schema_json_str(
        &mut self,
        _conn: Resource<DbConn>,
        _table_name: String,
    ) -> Result<String, PluginError> {
        Err(PluginError::HostDisabled(
            "vectordb feature flag is not enabled. The server owner must recompile with the vectordb feature flag enabled to expose this functionality".to_string(),
        ))
    }

    /// Adds data to a table (unavailable without vectordb feature).
    async fn add(
        &mut self,
        _conn: Resource<DbConn>,
        _table_name: String,
        _embedding_client: Resource<Client>,
        _embedding_model: String,
        _json_str_columns: Vec<String>,
        _to_embed_column_index: u32,
        _upsert_on: Option<String>,
    ) -> Result<bool, PluginError> {
        Err(PluginError::HostDisabled(
            "vectordb feature flag is not enabled".to_string(),
        ))
    }

    /// Delete data from a table (unavailable without vectordb feature).
    async fn delete(
        &mut self,
        _conn: Resource<DbConn>,
        _table_name: String,
        _predicate: String,
    ) -> Result<bool, PluginError> {
        Err(PluginError::HostDisabled(
            "vectordb feature flag is not enabled. The server owner must recompile with the vectordb feature flag enabled to expose this functionality".to_string(),
        ))
    }

    /// Drops a database connection resource.
    async fn drop(&mut self, conn: Resource<DbConn>) -> wasmtime::Result<()> {
        let _ = self.resources.delete(conn)?;
        Ok(())
    }
}

/// Implementation for vector database operations when the vectordb feature is enabled.
#[cfg(feature = "vectordb")]
impl vector_db::HostDbConnection for Host {
    /// Performs a similarity search in the vector database.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection resource
    /// * `similarity_config` - Configuration for similarity search
    /// * `embedding_client` - LLM client for generating embeddings
    /// * `embedding_model` - Name of the embedding model to use
    /// * `table_name` - Table to search in
    /// * `input` - Text to find similar entries for
    ///
    /// # Returns
    ///
    /// Vector of matching entries with similarity scores
    async fn similarity_search(
        &mut self,
        conn: Resource<DbConn>,
        similarity_config: SimilaritySearchConfig,
        embedding_client: Resource<Client>,
        embedding_model: String,
        table_name: String,
        input: String,
    ) -> Result<Vec<SimilarityResponse>, PluginError> {
        let mut db_conn = self
            .resources
            .get_mut(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        let results = db_conn
            .similarity_search(
                &mut self.resources,
                similarity_config,
                embedding_client,
                embedding_model,
                table_name,
                input,
            )
            .await?;
        Ok(results)
    }

    /// Retrieves a specific row by its ID from the vector database.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection resource
    /// * `table_name` - Table to query
    /// * `id_column` - Column containing the ID
    /// * `id_value` - Value of the ID to look for
    /// * `fields_returned` - Which fields to include in the result
    async fn get_row_by_id(
        &mut self,
        conn: Resource<DbConn>,
        table_name: String,
        id_column: String,
        id_value: String,
        fields_returned: Vec<String>,
    ) -> Result<Option<String>, PluginError> {
        let db_conn = self
            .resources
            .get_mut(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        let results = db_conn
            .get_row_by_id(&table_name, &id_column, &id_value, fields_returned)
            .await?;
        Ok(results)
    }

    /// Creates a new table in the vector database.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection resource
    /// * `table_name` - Name for the new table
    /// * `schema_json` - JSON string defining the table schema
    async fn create_table(
        &mut self,
        conn: Resource<DbConn>,
        table_name: String,
        schema_json: String,
    ) -> Result<bool, PluginError> {
        let db_conn = self
            .resources
            .get_mut(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        db_conn.create_table(&table_name, &schema_json).await
    }

    /// Retrieves a list of all table names in the database.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection resource
    #[cfg(feature = "vectordb")]
    async fn get_table_names(
        &mut self,
        conn: Resource<DbConn>,
    ) -> Result<Vec<String>, PluginError> {
        let db_conn = self
            .resources
            .get(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        let names = db_conn.get_table_names().await?;
        Ok(names)
    }

    /// Gets the schema of a table as a JSON string.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection resource
    /// * `table_name` - Name of the table to get schema for
    #[cfg(feature = "vectordb")]
    async fn get_table_schema_json_str(
        &mut self,
        conn: Resource<DbConn>,
        table_name: String,
    ) -> Result<String, PluginError> {
        let db_conn = self
            .resources
            .get(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        let schema = db_conn.get_table_schema_json_str(&table_name).await?;
        Ok(schema)
    }

    /// Adds or updates data in a table, generating embeddings for the specified column.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection resource
    /// * `table_name` - Table to add data to
    /// * `embedding_client` - LLM client for generating embeddings
    /// * `embedding_model` - Name of the embedding model to use
    /// * `json_str_columns` - JSON strings containing column data
    /// * `to_embed_column_index` - Index of the column to generate embeddings for
    /// * `upsert_on` - Optional column name for upsert operations
    #[cfg(feature = "vectordb")]
    async fn add(
        &mut self,
        conn: Resource<DbConn>,
        table_name: String,
        embedding_client: Resource<Client>,
        embedding_model: String,
        json_str_columns: Vec<String>,
        to_embed_column_index: u32,
        upsert_on: Option<String>,
    ) -> Result<bool, PluginError> {
        let db_conn = self
            .resources
            .get(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        let client = self
            .resources
            .get_mut(&embedding_client)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        db_conn
            .add(
                table_name,
                client,
                embedding_model,
                json_str_columns,
                to_embed_column_index,
                upsert_on,
            )
            .await?;
        Ok(true)
    }

    /// Deletes data from a table
    ///
    /// # Parameters
    /// * `table-name`: Name of the target table
    /// * `predicate`: The SQL predicate string to filter the rows to be deleted.
    #[cfg(feature = "vectordb")]
    async fn delete(
        &mut self,
        conn: Resource<DbConn>,
        table_name: String,
        predicate: String,
    ) -> Result<bool, PluginError> {
        let db_conn = self
            .resources
            .get(&conn)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?
            .clone();
        db_conn.delete(table_name, predicate).await
    }

    /// Drops a database connection resource.
    async fn drop(&mut self, conn: Resource<DbConn>) -> wasmtime::Result<()> {
        let _ = self.resources.delete(conn)?;
        Ok(())
    }
}

/// Implementation for LLM client operations.
impl open_a_i_like::HostClient for Host {
    /// Creates a chat completion using an LLM.
    ///
    /// # Arguments
    ///
    /// * `client` - LLM client resource
    /// * `config` - Configuration for the chat request
    ///
    /// # Returns
    ///
    /// The chat completion response from the LLM
    async fn chat_create(
        &mut self,
        client: Resource<Client>,
        config: ChatConfig,
    ) -> Result<ChatCompletion, PluginError> {
        let client: &mut Client = self
            .resources
            .get_mut(&client)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        chat_create(client, &config, &self.sse_stream_tx).await
    }

    /// Gets the dimensions of embeddings generated by a specific model.
    ///
    /// This is useful for configuring vector databases or other systems
    /// that need to know the embedding dimensions in advance.
    async fn get_embeddings_dimensions(
        &mut self,
        client: Resource<Client>,
        model: String,
    ) -> Result<u32, PluginError> {
        let res = self
            .embeddings_create_simple(
                client,
                model,
                EmbeddingInput::Str("This is a test".to_string()),
            )
            .await?;
        Ok(res[0].len() as u32)
    }

    /// Creates embeddings using simplified parameters.
    ///
    /// This is a convenience wrapper around the full embeddings_create method.
    async fn embeddings_create_simple(
        &mut self,
        client: Resource<Client>,
        model: String,
        input: EmbeddingInput,
    ) -> Result<Vec<Vec<f32>>, PluginError> {
        self.embeddings_create(client, model, input, None, None, None)
            .await
    }

    /// Creates embeddings with full parameter control.
    ///
    /// # Arguments
    ///
    /// * `client` - LLM client resource
    /// * `model` - Name of the embedding model to use
    /// * `input` - Text or tokens to embed
    /// * `encoding_format` - Optional format for the output embeddings
    /// * `user` - Optional user identifier for API tracking
    /// * `dimensions` - Optional output dimension size (model-dependent)
    async fn embeddings_create(
        &mut self,
        client: Resource<Client>,
        model: String,
        input: EmbeddingInput,
        encoding_format: Option<EncodingFormat>,
        user: Option<String>,
        dimensions: Option<u32>,
    ) -> Result<Vec<Vec<f32>>, PluginError> {
        let client: &mut Client = self
            .resources
            .get_mut(&client)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        let embeddings =
            embeddings_create(client, model, input, encoding_format, user, dimensions).await?;
        Ok(embeddings)
    }

    /// Drops an LLM client resource.
    async fn drop(&mut self, client: Resource<Client>) -> wasmtime::Result<()> {
        self.resources.delete(client)?;
        Ok(())
    }
}

/// Implementation for OpenAI configuration management.
impl open_a_i_like::HostOpenAIConfig for Host {
    /// Sets the organization ID on an OpenAI configuration.
    ///
    /// # Arguments
    ///
    /// * `resource` - OpenAI configuration resource
    /// * `org_id` - Organization ID to set
    ///
    /// # Returns
    ///
    /// New configuration resource with updated organization ID
    async fn with_org_id(
        &mut self,
        resource: Resource<OpenAIConfig>,
        org_id: String,
    ) -> Resource<OpenAIConfig> {
        let config: OpenAIConfig = self.resources.get(&resource).unwrap().clone();
        let new_config = OpenAIConfig(config.0.with_org_id(org_id));
        let _ = self.drop(resource).await; // drop old config
        self.resources.push(new_config).unwrap()
    }

    /// Sets the project ID on an OpenAI configuration.
    ///
    /// # Arguments
    ///
    /// * `resource` - OpenAI configuration resource
    /// * `project_id` - Project ID to set
    ///
    /// # Returns
    ///
    /// New configuration resource with updated project ID
    async fn with_project_id(
        &mut self,
        resource: Resource<OpenAIConfig>,
        project_id: String,
    ) -> Resource<OpenAIConfig> {
        let config: OpenAIConfig = self.resources.get(&resource).unwrap().clone();
        let new_config = OpenAIConfig(config.0.with_project_id(project_id));
        let _ = self.drop(resource).await; // drop old config
        self.resources.push(new_config).unwrap()
    }

    /// Sets the API key on an OpenAI configuration.
    ///
    /// # Arguments
    ///
    /// * `resource` - OpenAI configuration resource
    /// * `api_key` - API key to set
    ///
    /// # Returns
    ///
    /// New configuration resource with updated API key
    async fn with_api_key(
        &mut self,
        resource: Resource<OpenAIConfig>,
        api_key: String,
    ) -> Resource<OpenAIConfig> {
        let config: OpenAIConfig = self.resources.get(&resource).unwrap().clone();
        let new_config = OpenAIConfig(config.0.with_api_key(api_key));
        let _ = self.drop(resource).await; // drop old config
        self.resources.push(new_config).unwrap()
    }

    /// Sets the API base URL on an OpenAI configuration.
    ///
    /// # Arguments
    ///
    /// * `resource` - OpenAI configuration resource
    /// * `api_base` - Base URL to set
    ///
    /// # Returns
    ///
    /// New configuration resource with updated base URL
    async fn with_api_base(
        &mut self,
        resource: Resource<OpenAIConfig>,
        api_base: String,
    ) -> Resource<OpenAIConfig> {
        let config: OpenAIConfig = self.resources.get(&resource).unwrap().clone();
        let new_config = OpenAIConfig(config.0.with_api_base(api_base));
        let _ = self.drop(resource).await; // drop old config
        self.resources.push(new_config).unwrap()
    }

    /// Drops an OpenAI configuration resource.
    async fn drop(&mut self, resource: Resource<OpenAIConfig>) -> wasmtime::Result<()> {
        self.resources.delete(resource)?;
        Ok(())
    }
}

/// Implementation for chat session management.
impl open_a_i_like::HostChatSession for Host {
    /// Creates a new chat session.
    async fn new(
        &mut self,
        config: open_a_i_like::ChatConfig,
        client: Resource<Client>,
        generate_title: bool,
    ) -> Resource<ChatSession> {
        let client = self
            .resources
            .delete(client)
            .expect("Client resource should exist");

        let chat_session = ChatSession::new(config, client, generate_title);
        self.resources.push(chat_session).unwrap()
    }

    async fn session_id(&mut self, session: Resource<ChatSession>) -> String {
        let chat_session = self
            .resources
            .get(&session)
            .expect("Chat session resource should exist");

        chat_session.session_id()
    }

    async fn session_title(&mut self, session: Resource<ChatSession>) -> Option<String> {
        let chat_session = self
            .resources
            .get(&session)
            .expect("Chat session resource should exist");

        chat_session.session_title()
    }

    async fn config(&mut self, session: Resource<ChatSession>) -> open_a_i_like::ChatConfig {
        let chat_session = self
            .resources
            .get(&session)
            .expect("Chat session resource should exist");

        chat_session.config()
    }

    async fn enable_streaming(&mut self, session: Resource<ChatSession>) {
        let chat_session = self
            .resources
            .get_mut(&session)
            .expect("Chat session resource should exist");

        chat_session.enable_streaming();
    }

    async fn disable_streaming(&mut self, session: Resource<ChatSession>) {
        let chat_session = self
            .resources
            .get_mut(&session)
            .expect("Chat session resource should exist");

        chat_session.disable_streaming();
    }

    async fn set_tool_choice(
        &mut self,
        session: Resource<ChatSession>,
        tool_choice: Option<ToolSelection>,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .expect("Chat session resource should exist");

        chat_session.set_tool_choice(tool_choice)
    }

    async fn messages(&mut self, session: Resource<ChatSession>) -> Vec<open_a_i_like::Message> {
        let chat_session = self
            .resources
            .get(&session)
            .expect("Chat session resource should exist");

        chat_session.messages()
    }

    async fn set_messages(
        &mut self,
        session: Resource<ChatSession>,
        messages: Vec<open_a_i_like::Message>,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.set_messages(messages)
    }

    async fn chat(
        &mut self,
        session: Resource<ChatSession>,
        content: String,
    ) -> Result<open_a_i_like::ChatCompletion, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.chat(content, &self.sse_stream_tx).await
    }

    async fn add_tool(
        &mut self,
        session: Resource<ChatSession>,
        tool_schema: String,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.add_tool(tool_schema)
    }

    async fn remove_tool(
        &mut self,
        session: Resource<ChatSession>,
        tool_name: String,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.remove_tool(tool_name)
    }

    async fn remove_all_tools(
        &mut self,
        session: Resource<ChatSession>,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.remove_all_tools()
    }

    async fn add_message(
        &mut self,
        session: Resource<ChatSession>,
        message: open_a_i_like::Message,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.add_message(message)
    }

    async fn send_message(
        &mut self,
        session: Resource<ChatSession>,
        message: open_a_i_like::Message,
    ) -> Result<open_a_i_like::ChatCompletion, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session
            .send_message(message, &self.sse_stream_tx)
            .await
    }

    async fn send(
        &mut self,
        session: Resource<ChatSession>,
    ) -> Result<open_a_i_like::ChatCompletion, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.send(&self.sse_stream_tx).await
    }

    async fn fork_at(
        &mut self,
        session: Resource<ChatSession>,
        idx: u64,
    ) -> Result<Resource<ChatSession>, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        let new_session = chat_session.fork_at(idx)?;
        let session_resource = self
            .resources
            .push(new_session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;
        Ok(session_resource)
    }

    async fn set_response_schema(
        &mut self,
        session: Resource<ChatSession>,
        schema: Option<String>,
    ) -> Result<bool, PluginError> {
        let chat_session = self
            .resources
            .get_mut(&session)
            .map_err(|e| PluginError::ResourceError(e.to_string()))?;

        chat_session.set_response_schema(schema)
    }

    /// Drops a chat session resource.
    async fn drop(&mut self, session: Resource<ChatSession>) -> wasmtime::Result<()> {
        self.resources.delete(session)?;
        Ok(())
    }
}
