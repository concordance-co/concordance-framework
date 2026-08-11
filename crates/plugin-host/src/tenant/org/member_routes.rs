use crate::{
    server::AppState,
    tenant::{auth::AuthenticatedUser, org::Organization},
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateOrgRequest {
    pub name: String,
}

#[derive(Serialize)]
pub struct CreateOrgResponse {
    pub id: String,
    pub name: String,
}

/// Create a new organization
pub async fn org_register(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(req): Json<CreateOrgRequest>,
) -> impl IntoResponse {
    // Check if organization name already exists
    {
        let organizations = state.tenant_state.organizations.read().await;
        let name_exists = organizations.contains_key(&req.name);
        if name_exists {
            return (
                StatusCode::CONFLICT,
                "Organization with that name already exists".to_string(),
            )
                .into_response();
        }
    }

    // Check if organization name conflicts with a username
    {
        let users = state.tenant_state.users.read().await;
        if users.contains_key(&req.name) {
            return (
                StatusCode::CONFLICT,
                "Organization name conflicts with an existing username".to_string(),
            )
                .into_response();
        }
    }

    // Generate a unique ID for the organization
    let org_id = Uuid::new_v4().to_string();

    // Create the new organization
    let organization = Organization {
        id: org_id,
        name: req.name,
        created_at: Utc::now(),
        config: crate::tenant::org::OrganizationConfig {
            total_storage_quota_bytes: 1_000_000_000, // 1GB default
            max_users: 10,
            max_plugins: 100,
            max_pipelines: 100,
            shared_environment: HashMap::new(),
        },
        member_ids: vec![auth_user.user.username.clone()],
        member_roles: HashMap::from([(
            auth_user.user.username.clone(),
            crate::tenant::Role::Owner,
        )]),
    };

    // Add the organization to the system
    {
        let mut organizations = state.tenant_state.organizations.write().await;
        organizations.insert(organization.name.clone(), organization.clone());
    }

    // Add the organization to the user's list
    {
        let mut users = state.tenant_state.users.write().await;
        if let Some(user) = users.get_mut(&auth_user.user.username) {
            user.organization_ids.push(organization.name.clone());
        }
    }

    // Create organization directory structure
    let _ = crate::tenant::org::fs::organization_path(&organization.name);

    // Return success
    (
        StatusCode::CREATED,
        Json(CreateOrgResponse {
            id: organization.id,
            name: organization.name,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct AddUserToOrgRequest {
    pub username: String,
    pub org_name: String,
    pub role: crate::tenant::Role,
}

/// Add an existing user to an organization
pub async fn org_add_user(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Json(req): Json<AddUserToOrgRequest>,
) -> impl IntoResponse {
    // Check if the organization exists
    let mut organizations = state.tenant_state.organizations.write().await;
    let org = match organizations.get_mut(&req.org_name) {
        Some(org) => org,
        None => {
            return (StatusCode::NOT_FOUND, "Organization not found").into_response();
        }
    };

    // Check if auth user has permission (must be admin or owner)
    let user_role = org
        .member_roles
        .get(&auth_user.user.username)
        .cloned()
        .unwrap_or(crate::tenant::Role::Viewer);

    if user_role != crate::tenant::Role::Admin && user_role != crate::tenant::Role::Owner {
        return (
            StatusCode::FORBIDDEN,
            "You don't have permission to add users to this organization",
        )
            .into_response();
    }

    // Check that the role doesn't exceed that of the creator
    if req.role > user_role {
        return (
            StatusCode::FORBIDDEN,
            "You can't assign a role higher than your own",
        )
            .into_response();
    }

    // Check if user already in organization
    if org.member_ids.contains(&req.username) {
        return (
            StatusCode::BAD_REQUEST,
            "User is already a member of this organization",
        )
            .into_response();
    }

    // Verify the user exists
    let mut users = state.tenant_state.users.write().await;
    if !users.contains_key(&req.username) {
        return (StatusCode::NOT_FOUND, "User not found").into_response();
    }

    // Add user to organization
    org.member_ids.push(req.username.clone());
    org.member_roles.insert(req.username.clone(), req.role);

    // Add organization to user's list
    if let Some(user) = users.get_mut(&req.username) {
        user.organization_ids.push(req.org_name.clone());
    }

    // Return success
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "User added to organization successfully"
        })),
    )
        .into_response()
}
