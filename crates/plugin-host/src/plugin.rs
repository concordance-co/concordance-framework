//! Module for plugin execution and management.
use crate::injector::metadata_to_id;
use crate::server::SseStreamTx;
use crate::tenant::user::User;
use crate::tenant::SyncOrganizationRegistry;
use crate::{
    host::HostHolder,
    injector::{Injector, InjectorPre, Metadata},
    routes::PipelineJob,
};
use anyhow::Result;
use axum::http::StatusCode;
use std::collections::HashSet;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use wasmtime::{
    component::{Component, Linker},
    Config, Engine, Store,
};

pub type UserToPluginRef = HashMap<String, HashSet<String>>;
pub type SyncUserToPluginRef = Arc<RwLock<UserToPluginRef>>;
pub type PluginRegistry = HashMap<String, StringToStringWorker>;
pub type SyncPluginRegistry = Arc<RwLock<PluginRegistry>>;

pub async fn new_worker(
    wasm_bytes: &[u8],
    plugin_registry: SyncPluginRegistry,
    visible_plugins: SyncUserToPluginRef,
    orgs: SyncOrganizationRegistry,
) -> Result<(String, StringToStringWorker), String> {
    if wasm_bytes.is_empty() {
        return Err("No WASM file uploaded".to_string());
    }
    let worker =
        match StringToStringWorker::from_bytes(plugin_registry, visible_plugins, orgs, wasm_bytes)
            .await
        {
            Ok(worker) => worker,
            Err(e) => {
                tracing::warn!("Could not construct worker from uploaded WASM file");
                return Err(e.to_string());
            }
        };

    if worker.metadata.is_none() {
        tracing::warn!("Invalid metadata from uploaded WASM file");
        return Err("Invalid metadata from uploaded WASM file".to_string());
    }

    let plugin_id = metadata_to_id(worker.metadata.as_ref().unwrap());
    Ok((plugin_id, worker))
}

/// Execute a specific plugin
///
/// This function handles the actual execution of a plugin, retrieving it from
/// the plugin registry and running it with the provided input.
pub async fn run_plugin(
    plugin_manager: Arc<RwLock<HashMap<String, StringToStringWorker>>>,
    jobs: Arc<RwLock<Vec<PipelineJob>>>,
    plugin_id: String,
    input: serde_json::Value,
    job_id: Option<String>,
    user: Option<User>,
    sse_stream_tx: Option<SseStreamTx>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    tracing::info!(
        "Running plugin {plugin_id} with input: {} - streaming: {}",
        &input.to_string()[..10.min(input.to_string().len())],
        sse_stream_tx.is_some()
    );
    let plugin_manager = plugin_manager.read().await;
    if let Some(plugin) = plugin_manager.get(&plugin_id) {
        // Clone plugin before releasing the lock
        let mut plugin_clone = plugin.clone();
        plugin_clone.jobs = Some(jobs.clone());
        plugin_clone.job_id = job_id.map(|i| i.to_string());
        // Drop the lock before awaiting
        drop(plugin_manager);
        match plugin_clone
            .work(&input, user.as_ref(), sse_stream_tx)
            .await
        {
            Ok(val) => Ok(val),
            Err(e) => {
                tracing::warn!(
                    "Failed to execute plugin {}: {}",
                    plugin_clone.metadata.unwrap().name,
                    e
                );
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to execute plugin: {}", e),
                ))
            }
        }
    } else {
        tracing::warn!("Plugin not found: {}", plugin_id);
        Err((
            StatusCode::NOT_FOUND,
            format!("Plugin not found: {}", plugin_id),
        ))
    }
}

/// A WebAssembly component worker that processes JSON input to JSON output.
///
/// This worker loads and executes WebAssembly components that implement
/// the plugin interface for Concordance. It handles instantiation of
/// the component, metadata retrieval, and JSON processing.
#[derive(Clone)]
pub struct StringToStringWorker {
    pub raw_binary: Vec<u8>,
    /// The WebAssembly engine used to execute the component.
    engine: Engine,
    /// Pre-instantiated component for efficient repeated instantiation.
    injector_pre: InjectorPre<HostHolder>,
    /// Reference to other available plugins for cross-plugin communication.
    pub plugin_registry: SyncPluginRegistry,
    /// Reference to visible plugins for cross-plugin communication.
    pub visible_plugins: SyncUserToPluginRef,
    /// Registry of available organizations
    pub orgs: SyncOrganizationRegistry,
    /// Optional reference to pipeline jobs for status tracking.
    pub jobs: Option<Arc<RwLock<Vec<PipelineJob>>>>,
    /// Optional identifier for the current job being processed.
    pub job_id: Option<String>,
    /// Cached metadata from the plugin component.
    pub metadata: Option<Metadata>,
    pub plugin_id: Option<String>,
}

/// Debug implementation for StringToStringWorker that uses the metadata
/// to provide a more informative representation.
impl std::fmt::Debug for StringToStringWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let metadata = if let Some(ref metadata) = self.metadata {
            format!(
                "Plugin {{ name: {}, version: {}, author: {} }}",
                metadata.name, metadata.version, metadata.author
            )
        } else {
            "Plugin { metadata: None }".to_string()
        };

        f.debug_struct("StringToStringWorker")
            .field("metadata", &metadata)
            .field("job_id", &self.job_id)
            .finish()
    }
}

impl StringToStringWorker {
    /// Creates a new worker from WebAssembly component bytes.
    ///
    /// # Arguments
    ///
    /// * `other_plugins` - A reference to other available plugins
    /// * `module` - The WebAssembly component binary data
    ///
    /// # Returns
    ///
    /// A new `StringToStringWorker` instance ready to execute the component
    pub async fn from_bytes(
        plugin_registry: SyncPluginRegistry,
        visible_plugins: SyncUserToPluginRef,
        orgs: SyncOrganizationRegistry,
        module: &[u8],
    ) -> Result<StringToStringWorker> {
        let mut config = Config::new();
        // Enable component here.
        config.wasm_component_model(true);
        config.async_support(true);
        if let Ok(i) = std::env::var("WASM_DEBUG") {
            if i.parse::<u32>().unwrap_or(0) > 0 {
                println!("debug active");
                config.debug_info(true);
            } else {
                println!("debug inactive");
            }
        } else {
            println!("debug inactive");
        }

        let engine = Engine::new(&config)?;
        let mut linker: Linker<HostHolder> = Linker::new(&engine);
        let component = Component::from_binary(&engine, module)?;

        Injector::add_to_linker(&mut linker, |s| &mut s.host)?;
        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

        let pre = linker.instantiate_pre(&component)?;
        let injector_pre = InjectorPre::new(pre)?;

        let metadata = None;
        let mut worker = StringToStringWorker {
            raw_binary: module.to_vec(),
            engine,
            plugin_registry,
            visible_plugins,
            orgs,
            jobs: None,
            job_id: None,
            metadata,
            injector_pre,
            plugin_id: None,
        };
        worker.metadata = Some(worker.metadata().await?);
        worker.plugin_id = Some(metadata_to_id(worker.metadata.as_ref().unwrap()));
        Ok(worker)
    }

    /// Retrieves metadata from the plugin component.
    ///
    /// This method instantiates the component and calls the `get_metadata`
    /// function to retrieve information about the plugin's capabilities,
    /// requirements, and identity.
    ///
    /// # Returns
    ///
    /// The plugin's metadata
    pub async fn metadata(&self) -> Result<Metadata> {
        let state = HostHolder::new(
            "".to_string(),
            self.plugin_registry.clone(),
            self.visible_plugins.clone(),
            self.orgs.clone(),
            self.jobs.clone(),
            None,
            None,
            None,
        );
        let mut store = Store::new(&self.engine, state);
        let instance = self.injector_pre.instantiate_async(&mut store).await?;
        instance
            .plugin_injector_guest()
            .call_get_metadata(&mut store)
            .await
    }

    /// Processes JSON input through the plugin component.
    ///
    /// # Arguments
    ///
    /// * `value` - The JSON input to process
    ///
    /// # Returns
    ///
    /// The JSON output from the plugin, or an error if processing failed
    pub async fn work(
        &self,
        value: &serde_json::Value,
        user: Option<&User>,
        sse_stream_tx: Option<SseStreamTx>,
    ) -> Result<serde_json::Value> {
        tracing::debug!("plugin.work - streaming: {}", sse_stream_tx.is_some());
        let state = HostHolder::new(
            self.plugin_id.clone().unwrap(),
            self.plugin_registry.clone(),
            self.visible_plugins.clone(),
            self.orgs.clone(),
            self.jobs.clone(),
            self.job_id.clone(),
            user.cloned(),
            sse_stream_tx,
        );
        let mut store = Store::new(&self.engine, state);
        let instance = self.injector_pre.instantiate_async(&mut store).await?;

        let injector = instance
            .plugin_injector_guest()
            .json_to_json()
            .call_constructor(&mut store)
            .await?;

        let res = instance
            .plugin_injector_guest()
            .json_to_json()
            .call_work(&mut store, injector, &value.to_string())
            .await?;

        match res {
            Ok(result) => Ok(serde_json::from_str(&result)?),
            Err(err) => Err(err.into()),
        }
    }
}
