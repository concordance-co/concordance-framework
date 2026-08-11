use crate::server::AppState;
use crate::tenant::auth::AuthenticatedUser;
use crate::tenant::user::login::{hash_and_salt_password, verify_password};
use crate::tenant::user::UserHash;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// API Key structure with organization and user context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key_short: String,
    /// The API key value (hashed in storage)
    pub key_hash: String,
    /// User who owns this key
    pub userhash: UserHash,
    /// Name of this API key
    pub name: String,
    /// When this key was created
    pub created_at: DateTime<Utc>,
    /// When this key expires (if ever)
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether this key is active
    pub active: bool,
    /// Last time this key was used
    pub last_used: Option<DateTime<Utc>>,
    /// Permissions granted to this key (might be more restricted than the user)
    pub scopes: Vec<String>,
}

impl ApiKey {
    /// Generate a new API key
    pub fn new(userhash: &UserHash, name: &str) -> (Self, String) {
        let (key, key_hash) = generate_api_key();
        (
            Self {
                key_short: key[..10].to_string(),
                key_hash: key_hash.clone(),
                userhash: userhash.clone(),
                name: name.to_string(),
                created_at: Utc::now(),
                expires_at: None,
                active: true,
                last_used: None,
                scopes: vec!["*".to_string()], // Full access by default
            },
            key,
        )
    }

    /// Verify an API key
    pub fn verify(&self, key: &str) -> bool {
        self.active
            && self.expires_at.is_none_or(|exp| exp > Utc::now())
            && verify_api_key(key, &self.key_hash)
    }

    /// Mark this key as used
    pub fn mark_used(&mut self) {
        self.last_used = Some(Utc::now());
    }
}
/// Request body for creating a new API key
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    /// Name for the API key
    pub name: String,
    /// Optional expiration date
    pub expires_at: Option<DateTime<Utc>>,
    /// Optional scopes (permissions)
    pub scopes: Option<Vec<String>>,
}

/// Response for a newly created API key
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    /// The actual API key value (only returned once)
    pub api_key: String,
    /// Key metadata
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Generate a new API key for a user
pub async fn create_api_key(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> impl IntoResponse {
    let username = auth_user.user.username.clone();

    // Get the user's hash
    let mut users = state.tenant_state.users.write().await;
    let user = match users.get_mut(&username) {
        Some(user) => user,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to find user".to_string(),
            )
                .into_response();
        }
    };

    // Generate a new API key
    let (api_key, key_value) = ApiKey::new(&user.id, &req.name);

    // Set optional fields
    let api_key = ApiKey {
        expires_at: req.expires_at,
        scopes: req.scopes.unwrap_or_else(|| vec!["*".to_string()]),
        ..api_key
    };

    // Store the key
    user.api_keys.push(api_key.clone());
    state
        .tenant_state
        .api_key_to_user
        .write()
        .await
        .insert(api_key.key_hash.clone(), username.clone());

    // Return the API key to the user (this is the only time they'll see the full key)
    (
        StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            api_key: key_value,
            name: api_key.name,
            created_at: api_key.created_at,
            expires_at: api_key.expires_at,
        }),
    )
        .into_response()
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveApiKeyRequest {
    key_short: String,
}

/// remove an api key for a user
pub async fn remove_api_key(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(req): Json<RemoveApiKeyRequest>,
) -> impl IntoResponse {
    let username = auth_user.user.username.clone();

    // Update the user's API key
    let mut users = state.tenant_state.users.write().await;
    if let Some(user) = users.get_mut(&username) {
        if let Some(index) = user
            .api_keys
            .iter()
            .position(|k| k.key_short == req.key_short)
        {
            let api_key = user.api_keys.remove(index);
            let _ = state
                .tenant_state
                .api_key_to_user
                .write()
                .await
                .remove(&api_key.key_hash);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "message": "API key removed successfully"
                })),
            )
                .into_response()
        } else {
            (StatusCode::NOT_FOUND, "API key not found".to_string()).into_response()
        }
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to remove API key".to_string(),
        )
            .into_response()
    }
}

/// List all API keys for a user
pub async fn list_api_keys(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> impl IntoResponse {
    let username = auth_user.user.username.clone();

    // Get the user's API keys
    let users = state.tenant_state.users.read().await;
    let user = match users.get(&username) {
        Some(user) => user,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to find user".to_string(),
            )
                .into_response();
        }
    };

    // Return just the key_short identifiers and metadata
    let api_keys: Vec<serde_json::Value> = user
        .api_keys
        .iter()
        .map(|key| {
            serde_json::json!({
                "key_short": key.key_short,
                "name": key.name,
                "created_at": key.created_at,
                "expires_at": key.expires_at,
                "last_used": key.last_used,
                "active": key.active,
                "scopes": key.scopes
            })
        })
        .collect();

    (StatusCode::OK, Json(api_keys)).into_response()
}

/// Generate a secure API key
fn generate_api_key() -> (String, String) {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    use base64::prelude::*;
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let encoded_key = BASE64_STANDARD.encode(key);
    hash_api_key(&encoded_key)
}

/// Hash an API key for storage
fn hash_api_key(key: &str) -> (String, String) {
    let (hash, salt) = hash_and_salt_password(key);
    (format!("conc-{key}-{salt}"), hash)
}

/// Verify an API key against a hash
fn verify_api_key(key: &str, hash: &str) -> bool {
    let hashed_key = key.split("-").nth(1).unwrap_or_default();
    verify_password(hashed_key, hash)
}
