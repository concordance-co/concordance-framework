use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::auth::api_key::ApiKey;

pub mod daemon_routes;
pub mod fs;
pub mod login;
pub mod member_routes;
pub mod pipeline_routes;
pub mod plugin_routes;

pub use daemon_routes::*;
pub use member_routes::*;
pub use pipeline_routes::*;
pub use plugin_routes::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserHash(pub String);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthType {
    #[default]
    NoAuth,
    Basic {
        username: String,
        password: String,
    },
}

/// Represents a user belonging to an organization
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct User {
    /// Hash of the username
    pub id: UserHash,
    /// Unique user name
    pub username: String,
    /// Display name of the user
    pub name: String,
    /// Password hash for the user
    pub password_hash: String,
    /// When the user was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Organizations this user belongs to
    pub organization_ids: Vec<String>,
    /// User-specific configuration overrides
    pub config: UserConfig,
    /// Whether the user is active
    pub active: bool,
    /// API keys for programmatic access
    pub api_keys: Vec<ApiKey>,
    pub salt: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserConfig {
    /// User-specific environment variables
    pub environment_variables: HashMap<String, Value>,
}
