use crate::{
    server::AppState,
    tenant::{
        auth::{jwt::ConcordanceClaims, AuthenticatedUser},
        user::{
            login::{hash_and_salt_password, verify_password},
            AuthType, User, UserConfig, UserHash,
        },
    },
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use uuid;

#[derive(Deserialize)]
pub struct LoginRequest {
    #[serde(flatten)]
    pub auth: AuthType,
}

#[derive(Serialize)]
pub struct UserInfo {
    username: String,
    name: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    token: String,
    user: UserInfo,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub name: String,
}

/// Register a new user in the system
pub async fn user_register(
    State(state): State<AppState>,
    Json(registration): Json<RegisterRequest>,
) -> impl IntoResponse {
    // Check if the username already exists
    let mut users = state.tenant_state.users.write().await;
    if users.contains_key(&registration.username) {
        return (StatusCode::CONFLICT, "Username already exists".to_string()).into_response();
    }

    // Check if there is an organization with the same name as the username
    let organizations = state.tenant_state.organizations.read().await;
    if organizations.contains_key(&registration.username) {
        return (
            StatusCode::CONFLICT,
            "An organization with this name already exists".to_string(),
        )
            .into_response();
    }

    // Create the new user
    let now = chrono::Utc::now();
    let (password_hash, salt) = hash_and_salt_password(&registration.password);

    let user = User {
        id: UserHash(format!("user_{}", uuid::Uuid::new_v4())),
        username: registration.username.clone(),
        name: registration.name,
        password_hash,
        created_at: now,
        organization_ids: vec![],
        config: UserConfig::default(),
        active: true,
        api_keys: vec![],
        salt: salt.to_string(),
    };

    // Add user to the system
    users.insert(registration.username.clone(), user.clone());

    // Initialize empty plugin references for the user
    state
        .tenant_state
        .user_plugin_refs
        .write()
        .await
        .insert(registration.username.clone(), HashSet::new());

    state
        .tenant_state
        .user_visible_plugins
        .write()
        .await
        .insert(registration.username.clone(), HashSet::new());

    // Generate a token for the new user
    match state
        .tenant_state
        .jwt_manager
        .create_token(ConcordanceClaims::new(&user.username, 24))
    {
        Ok(token) => {
            // Return login response
            (
                StatusCode::CREATED,
                Json(LoginResponse {
                    token,
                    user: UserInfo {
                        username: user.username,
                        name: user.name,
                    },
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to generate token: {}", e),
        )
            .into_response(),
    }
}

/// Login a user with some form of authentication
pub async fn user_login(
    State(state): State<AppState>,
    Json(credentials): Json<LoginRequest>,
) -> impl IntoResponse {
    let mut user = None;
    if state.tenant_state.enforce_auth {
        let authed = match credentials.auth {
            AuthType::NoAuth => false,
            AuthType::Basic { username, password } => {
                let users = state.tenant_state.users.read().await;
                if let Some(found_user) = users.get(&username) {
                    user = Some(found_user.clone());
                    verify_password(&password, &found_user.password_hash)
                } else {
                    false
                }
            }
        };

        if !authed || user.is_none() {
            return (
                StatusCode::UNAUTHORIZED,
                "Authorization is required or invalid credentials".to_string(),
            )
                .into_response();
        }
    } else {
        user = Some(User::default());
    }

    let user = user.unwrap();
    // Sign the token
    match state
        .tenant_state
        .jwt_manager
        .create_token(ConcordanceClaims::new(&user.username, 24))
    {
        Ok(token) => {
            // Return login response
            (
                StatusCode::OK,
                Json(LoginResponse {
                    token,
                    user: UserInfo {
                        username: user.username,
                        name: user.name,
                    },
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to generate token: {}", e),
        )
            .into_response(),
    }
}

/// Payload for adding a user environment variable
#[derive(Debug, Deserialize)]
pub struct UserEnvVarPayload {
    /// Key for the environment variable
    pub key: String,
    /// Value of the environment variable
    pub value: Value,
}

/// Adds an environment variable for a user
pub async fn user_add_env_var(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(payload): Json<UserEnvVarPayload>,
) -> impl IntoResponse {
    // Get the user
    let mut users = state.tenant_state.users.write().await;
    let user = match users.get_mut(&auth_user.user.username) {
        Some(user) => user,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "User not found"
                })),
            )
                .into_response();
        }
    };

    // Add or update the environment variable for the user
    if payload.value.as_null().is_some() {
        user.config.environment_variables.remove(&payload.key);
    } else {
        user.config
            .environment_variables
            .insert(payload.key.clone(), payload.value.clone());
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": format!("Environment variable '{}' added", payload.key)
        })),
    )
        .into_response()
}

/// Get information about the current user
pub async fn user_info(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
) -> impl IntoResponse {
    // Gather organization roles
    let org_roles = {
        let orgs = state.tenant_state.organizations.read().await;
        let mut roles = std::collections::HashMap::new();

        for org_id in &auth_user.user.organization_ids {
            if let Some(org) = orgs.get(org_id) {
                if let Some(role) = org.member_roles.get(&auth_user.user.username) {
                    roles.insert(org_id.clone(), role.clone());
                }
            }
        }
        roles
    };

    // Count user's plugins
    let plugin_count = state
        .tenant_state
        .user_plugin_refs
        .read()
        .await
        .get(&auth_user.user.username)
        .map(|plugins| plugins.len())
        .unwrap_or(0);

    // Count visible plugins
    let visible_plugin_count = state
        .tenant_state
        .user_visible_plugins
        .read()
        .await
        .get(&auth_user.user.username)
        .map(|plugins| plugins.len())
        .unwrap_or(0);

    // Return the user information with organization roles and plugin counts
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "username": auth_user.user.username,
            "name": auth_user.user.name,
            "created_at": auth_user.user.created_at,
            "organization_map": org_roles,
            "active": auth_user.user.active,
            "config": auth_user.user.config,
            "plugins": {
                "created": plugin_count,
                "visible": visible_plugin_count
            }
        })),
    )
        .into_response()
}
