use super::host_impl::Host;
use super::writeable_path;
use crate::plugin::{SyncPluginRegistry, SyncUserToPluginRef};
use crate::routes::PipelineJob;
use crate::server::SseStreamTx;
use crate::tenant::user::fs::user_path;
use crate::tenant::user::User;
use crate::tenant::SyncOrganizationRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{DirPerms, FilePerms, IoView, WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

/// Plugins host holder for WASI context.
pub struct HostHolder {
    pub host: Host,
    wasi: WasiCtx,
    http: WasiHttpCtx,
}

impl HostHolder {
    /// Constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plugin_id: String,
        plugin_registry: SyncPluginRegistry,
        visible_plugins: SyncUserToPluginRef,
        orgs: SyncOrganizationRegistry,
        jobs: Option<Arc<RwLock<Vec<PipelineJob>>>>,
        job_id: Option<String>,
        user: Option<User>,
        sse_stream_tx: Option<SseStreamTx>,
    ) -> Self {
        tracing::debug!("new hostholder - streaming: {}", sse_stream_tx.is_some());
        let concordance_dir = if let Some(ref user) = user {
            user_path(None, &user.username)
        } else {
            writeable_path()
        };
        let wasi = WasiCtxBuilder::new()
            .inherit_network()
            .inherit_stderr()
            .inherit_stdout()
            .allow_ip_name_lookup(true)
            .preopened_dir(concordance_dir, ".", DirPerms::all(), FilePerms::all())
            .unwrap()
            .build();

        Self {
            host: Host::new(
                plugin_id,
                plugin_registry,
                visible_plugins,
                orgs,
                jobs,
                job_id,
                user,
                sse_stream_tx,
            ),
            wasi,
            http: WasiHttpCtx::new(),
        }
    }
}

impl IoView for HostHolder {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.host.resources
    }
}

impl WasiView for HostHolder {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl WasiHttpView for HostHolder {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
}
