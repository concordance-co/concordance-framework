use crate::{server::AppState, tenant::User};
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use reqwest::header;

use super::user::login::hash_with_salt;

pub mod api_key;
pub mod jwt;

/// User authentication information
pub struct AuthenticatedUser {
    /// The authenticated user
    pub user: User,
}

/// Axum extractor to get authenticated user from a request
impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !state.tenant_state.enforce_auth {
            return Ok(AuthenticatedUser {
                user: User::default(),
            });
        }
        // First try API key authentication
        if let Some(api_key) = parts.headers.get("X-API-Key").and_then(|v| v.to_str().ok()) {
            let mut api_key = api_key.split("-");
            let key_value = api_key.nth(1);
            let salt = api_key.next();
            if let (Some(val), Some(salt)) = (key_value, salt) {
                let hashed_key = hash_with_salt(val, salt);
                return authenticate_with_api_key(&hashed_key, state).await;
            }
        }

        // Then try bearer token authentication
        if let Some(auth_header) = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        {
            if let Some(token) = auth_header.strip_prefix("Bearer ") {
                return authenticate_with_token(token, state).await;
            }
        }

        Err((
            StatusCode::UNAUTHORIZED,
            "Valid authentication required".to_string(),
        ))
    }
}

/// Authenticate using an API key
async fn authenticate_with_api_key(
    api_key: &str,
    app_state: &AppState,
) -> Result<AuthenticatedUser, (StatusCode, String)> {
    // Find user by API key
    let users = app_state.tenant_state.users.read().await;
    let api_key_to_user = app_state.tenant_state.api_key_to_user.read().await;

    let user_id = api_key_to_user.get(api_key).cloned().ok_or((
        StatusCode::UNAUTHORIZED,
        "Invalid API key - no associated user".to_string(),
    ))?;

    let user = users
        .get(&user_id)
        .cloned()
        .ok_or((StatusCode::UNAUTHORIZED, "No user found".to_string()))?;

    if !user.active {
        return Err((StatusCode::UNAUTHORIZED, "User is not active".to_string()));
    }

    if !user.api_keys.iter().any(|key| key.key_hash == api_key) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Invalid API key - not found in user's keys".to_string(),
        ));
    };

    Ok(AuthenticatedUser { user })
}

/// Authenticate using a JWT token
async fn authenticate_with_token(
    token: &str,
    app_state: &AppState,
) -> Result<AuthenticatedUser, (StatusCode, String)> {
    // Verify the token
    let claims = match app_state.tenant_state.jwt_manager.verify_token(token) {
        Ok(claims) => claims,
        Err(e) => {
            return Err((StatusCode::UNAUTHORIZED, format!("Invalid token: {}", e)));
        }
    };

    // Get the user from the database
    let users = app_state.tenant_state.users.read().await;
    let user = users
        .get(&claims.user_id)
        .cloned()
        .ok_or((StatusCode::UNAUTHORIZED, "User not found".to_string()))?;

    if !user.active {
        return Err((
            StatusCode::FORBIDDEN,
            "User account is inactive".to_string(),
        ));
    }

    Ok(AuthenticatedUser { user })
}
