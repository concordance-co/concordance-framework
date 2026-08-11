use crate::tenant::Role;
use chrono;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub mod fs;
/// API routes related to organization management
pub mod member_routes;
pub mod resource_routes;

pub use member_routes::*;
pub use resource_routes::*;

/// Represents an organization in the system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Organization {
    /// Unique identifier for the organization
    pub id: String,
    /// Display name of the organization
    pub name: String,
    /// When the organization was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Organization-wide configuration
    pub config: OrganizationConfig,
    /// List of member ids in the organization
    pub member_ids: Vec<String>,
    /// Map of member usernames to roles in the organization
    pub member_roles: HashMap<String, Role>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedEnvVar {
    pub min_role: Role,
    pub value: Value,
}

/// Configuration for an organization
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrganizationConfig {
    /// Total storage quota for the organization (bytes)
    pub total_storage_quota_bytes: u64,
    /// Maximum number of users allowed
    pub max_users: usize,
    /// Maximum number of plugins allowed
    pub max_plugins: usize,
    /// Maximum number of pipelines allowed
    pub max_pipelines: usize,
    /// Shared environment variables for all organization users and the role
    /// necessary to use them
    pub shared_environment: HashMap<String, SharedEnvVar>,
}
