use crate::pipeline::run_pipeline;
use crate::plugin::run_plugin;
use crate::plugin::StringToStringWorker;
use crate::server::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use tracing::{debug, error, info, trace, warn};

pub type DaemonRegistry = HashMap<String, Daemon>;
pub type SyncDaemonRegistry = Arc<RwLock<DaemonRegistry>>;
pub type IdToDaemonRef = HashMap<String, HashSet<String>>;
pub type SyncIdToDaemonRef = Arc<RwLock<IdToDaemonRef>>;

/// Defines the type of task a daemon will execute.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum DaemonKind {
    /// A pipeline execution daemon with pipeline ID
    Pipeline(String),
    /// A plugin execution daemon with plugin ID
    Plugin(String),
}

/// Configuration for a daemon process.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DaemonConfig {
    /// Type of daemon (pipeline or plugin)
    pub kind: DaemonKind,
    /// Input data to be passed to the pipeline or plugin
    pub input: serde_json::Value,
    /// How often the daemon should run (in seconds)
    pub frequency_seconds: u64,
    /// Whether the daemon is currently enabled
    pub enabled: bool,
    /// Human-readable name for the daemon
    pub name: String,
    /// Optional description of the daemon's purpose
    pub description: Option<String>,
}

/// Represents an active daemon instance in the system.
#[derive(Debug, Deserialize, Serialize)]
pub struct Daemon {
    /// Unique identifier for the daemon
    pub id: String,
    /// Configuration settings for this daemon
    pub config: DaemonConfig,
    /// When the daemon was last executed
    pub last_run: Option<chrono::DateTime<chrono::Utc>>,
    /// When the daemon is scheduled to run next
    pub next_run: chrono::DateTime<chrono::Utc>,
    /// Current operational status of the daemon
    pub status: DaemonStatus,
    /// Result from the last execution (if any)
    pub last_result: Option<serde_json::Value>,
    /// Count of consecutive execution errors
    pub error_count: u32,
    #[serde(skip)]
    task_handle: Option<JoinHandle<()>>,
    #[serde(skip)]
    plugins: Arc<RwLock<HashMap<String, StringToStringWorker>>>,
}

impl Clone for Daemon {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            config: self.config.clone(),
            last_run: self.last_run,
            next_run: self.next_run,
            status: self.status.clone(),
            last_result: self.last_result.clone(),
            error_count: self.error_count,
            task_handle: None, // Don't clone the task handle
            plugins: self.plugins.clone(),
        }
    }
}

/// Represents the operational status of a daemon.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum DaemonStatus {
    /// Daemon is running normally
    Active,
    /// Daemon has been manually paused
    Paused,
    /// Daemon is in error state (too many failures)
    Error,
    /// Daemon is starting up
    Initializing,
}

impl Daemon {
    /// Creates a new daemon instance with the given ID and configuration.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for the daemon
    /// * `config` - Configuration settings for the daemon
    /// * `plugins` - Plugins associated with the daemon
    pub fn new(
        id: String,
        config: DaemonConfig,
        plugins: Arc<RwLock<HashMap<String, StringToStringWorker>>>,
    ) -> Self {
        let now = chrono::Utc::now();
        info!(
            daemon_id = %id,
            daemon_type = ?config.kind,
            frequency = %config.frequency_seconds,
            "Creating new daemon"
        );
        Self {
            id,
            next_run: now + chrono::Duration::seconds(config.frequency_seconds as i64),
            config,
            last_run: None,
            status: DaemonStatus::Initializing,
            last_result: None,
            error_count: 0,
            task_handle: None,
            plugins,
        }
    }

    /// Determines if the daemon should be executed at the current time.
    ///
    /// Returns true if the daemon is enabled, not paused, and scheduled time has passed.
    pub fn should_run(&self) -> bool {
        let now = chrono::Utc::now();
        let should_run =
            self.config.enabled && self.status != DaemonStatus::Paused && now >= self.next_run;

        trace!(
            daemon_id = %self.id,
            enabled = %self.config.enabled,
            status = ?self.status,
            next_run = %self.next_run,
            now = %now,
            "Checking if daemon should run: {}",
            should_run
        );

        should_run
    }

    /// Executes the daemon's task (pipeline or plugin).
    ///
    /// # Arguments
    /// * `state` - Application state containing necessary runtime components
    ///
    /// # Returns
    /// * `Ok(Value)` - Result of the successful execution
    /// * `Err(String)` - Error message if execution failed
    pub async fn run(&mut self, state: &AppState) -> Result<serde_json::Value, String> {
        let now = chrono::Utc::now();
        self.last_run = Some(now);
        info!(
            daemon_id = %self.id,
            daemon_kind = ?self.config.kind,
            "Executing daemon task"
        );

        let result: Result<serde_json::Value, String> = match &self.config.kind {
            DaemonKind::Plugin(plugin_id) => {
                debug!(
                    daemon_id = %self.id,
                    plugin_id = %plugin_id,
                    "Running plugin for daemon"
                );
                // Run plugin
                match run_plugin(
                    self.plugins.clone(),
                    state.jobs.clone(),
                    plugin_id.to_string(),
                    self.config.input.clone(),
                    None,
                    None,
                    None,
                )
                .await
                {
                    Ok(output) => {
                        debug!(
                            daemon_id = %self.id,
                            plugin_id = %plugin_id,
                            "Plugin execution successful"
                        );
                        Ok(output)
                    }
                    Err(e) => {
                        error!(
                            daemon_id = %self.id,
                            plugin_id = %plugin_id,
                            error = %e.1,
                            "Plugin execution failed"
                        );
                        Err(format!("Plugin execution error: {}", e.1))
                    }
                }
            }
            DaemonKind::Pipeline(pipeline_id) => {
                debug!(
                    daemon_id = %self.id,
                    pipeline_id = %pipeline_id,
                    "Running pipeline for daemon"
                );

                let Some(pipeline) = state
                    .pipeline_manager
                    .read()
                    .await
                    .get(pipeline_id)
                    .cloned()
                else {
                    error!(
                        daemon_id = %self.id,
                        pipeline_id = %pipeline_id,
                        "Pipeline not found"
                    );
                    return Err(format!("Pipeline not found: {}", pipeline_id));
                };
                // Run pipeline
                match run_pipeline(
                    self.plugins.clone(),
                    state.jobs.clone(),
                    pipeline,
                    self.config.input.clone(),
                    None,
                    None,
                    None,
                )
                .await
                {
                    Ok(output) => {
                        debug!(
                            daemon_id = %self.id,
                            pipeline_id = %pipeline_id,
                            "Pipeline execution successful"
                        );
                        Ok(output.1)
                    }
                    Err(e) => {
                        error!(
                            daemon_id = %self.id,
                            pipeline_id = %pipeline_id,
                            error = %e.1,
                            "Pipeline execution failed"
                        );
                        Err(format!("Pipeline execution error: {}", e.1))
                    }
                }
            }
        };

        // Update status based on result
        match &result {
            Ok(output) => {
                debug!(
                    daemon_id = %self.id,
                    "Daemon execution successful, updating status"
                );
                self.status = DaemonStatus::Active;
                self.last_result = Some(output.clone());
                self.error_count = 0;
            }
            Err(err_msg) => {
                warn!(
                    daemon_id = %self.id,
                    error_count = %self.error_count,
                    error = %err_msg,
                    "Daemon execution failed, incrementing error count"
                );
                self.error_count += 1;
                if self.error_count >= 3 {
                    warn!(
                        daemon_id = %self.id,
                        error_count = %self.error_count,
                        "Daemon reached error threshold, setting status to Error"
                    );
                    self.status = DaemonStatus::Error;
                }
            }
        }

        // Schedule next run
        self.next_run =
            chrono::Utc::now() + chrono::Duration::seconds(self.config.frequency_seconds as i64);
        info!(
            daemon_id = %self.id,
            next_run = %self.next_run,
            "Scheduled next daemon execution"
        );

        result
    }

    /// Pauses the daemon, preventing it from running on schedule.
    pub fn pause(&mut self) {
        info!(daemon_id = %self.id, "Pausing daemon");
        self.status = DaemonStatus::Paused;

        // Cancel the task if it exists
        if let Some(handle) = self.task_handle.take() {
            debug!(daemon_id = %self.id, "Aborting daemon background task");
            handle.abort();
        }
    }

    /// Resumes a paused daemon.
    ///
    /// If the daemon had previously accumulated too many errors,
    /// it will resume in Error state rather than Active.
    pub async fn resume(&mut self, state: Arc<AppState>) {
        if self.status == DaemonStatus::Paused || self.status == DaemonStatus::Initializing {
            self.status = if self.error_count >= 3 {
                info!(
                    daemon_id = %self.id,
                    error_count = %self.error_count,
                    "Resuming daemon in Error state due to error count"
                );
                DaemonStatus::Error
            } else {
                info!(daemon_id = %self.id, "Resuming daemon in Active state");
                DaemonStatus::Active
            };

            // Start a new background task for this daemon
            let daemon_id = self.id.clone();
            let state_clone = Arc::clone(&state);

            // Cancel any existing task before creating a new one
            if let Some(handle) = self.task_handle.take() {
                debug!(daemon_id = %self.id, "Aborting existing daemon task before resuming");
                handle.abort();
            }

            info!(daemon_id = %self.id, "Starting daemon background task");
            self.task_handle = Some(tokio::spawn(async move {
                let mut interval = {
                    let daemons = state_clone.daemons.read().await;
                    let Some(daemon) = daemons.get(&daemon_id) else {
                        error!(daemon_id = %daemon_id, "Daemon not found when starting background task");
                        return;
                    };
                    let frequency = daemon.config.frequency_seconds;
                    debug!(
                        daemon_id = %daemon_id,
                        frequency_seconds = %frequency,
                        "Creating interval for daemon task"
                    );
                    tokio::time::interval(std::time::Duration::from_secs(frequency))
                };

                debug!(daemon_id = %daemon_id, "Daemon background task started");
                loop {
                    trace!(daemon_id = %daemon_id, "Waiting for next interval tick");
                    interval.tick().await;
                    trace!(daemon_id = %daemon_id, "Interval tick received");

                    // Check if daemon should run
                    let mut daemons = state_clone.daemons.write().await;
                    let should_run = match daemons.get(&daemon_id) {
                        Some(daemon) => daemon.should_run(),
                        None => {
                            info!(daemon_id = %daemon_id, "Daemon no longer exists, exiting background task");
                            break; // Daemon no longer exists, exit loop
                        }
                    };

                    if should_run {
                        debug!(daemon_id = %daemon_id, "Daemon scheduled to run now");
                        if let Some(daemon) = daemons.get_mut(&daemon_id) {
                            match daemon.run(&state_clone).await {
                                Ok(_) => {
                                    trace!(daemon_id = %daemon_id, "Daemon run completed successfully")
                                }
                                Err(e) => {
                                    warn!(daemon_id = %daemon_id, error = %e, "Daemon run failed")
                                }
                            }
                        }
                    }
                }
            }));
        } else {
            debug!(
                daemon_id = %self.id,
                status = ?self.status,
                "Not resuming daemon as it's not in Paused or Initializing state"
            );
        }
    }
}
