use crate::routes::ServerMetadata;
use crate::{
    server::AppState,
    tenant::{auth::AuthenticatedUser, Role},
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::Value;

/// Payload for adding a shared environment variable
#[derive(Debug, Deserialize)]
pub struct SharedEnvVarPayload {
    /// Key for the environment variable
    pub key: String,
    /// Value of the environment variable
    pub value: Value,
    /// Minimum role required to use this environment variable
    pub min_role: Role,
}

/// Adds a shared environment variable for an organization
pub async fn org_add_env_var(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(org_name): Path<String>,
    Json(payload): Json<SharedEnvVarPayload>,
) -> impl IntoResponse {
    // Check if user has admin privileges in the organization
    let user_role = {
        let orgs = state.tenant_state.organizations.read().await;
        if let Some(org) = orgs.get(&org_name) {
            org.member_roles.get(&auth_user.user.username).cloned()
        } else {
            None
        }
    };

    match user_role {
        Some(role) if role >= Role::Manager => {
            // User has manager or higher privileges - can add shared env vars
            let mut orgs = state.tenant_state.organizations.write().await;
            if let Some(org) = orgs.get_mut(&org_name) {
                // Add or update the environment variable
                if payload.value.is_null() {
                    org.config.shared_environment.remove(&payload.key);
                } else {
                    org.config.shared_environment.insert(
                        payload.key.clone(),
                        super::SharedEnvVar {
                            min_role: payload.min_role,
                            value: payload.value.clone(),
                        },
                    );
                }

                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "success",
                        "message": format!("Environment variable '{}' updated", payload.key)
                    })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": "Organization not found"
                    })),
                )
                    .into_response()
            }
        }
        Some(_) => {
            // User doesn't have sufficient privileges
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "Insufficient privileges to add shared environment variables"
                })),
            )
                .into_response()
        }
        None => {
            // User not in organization
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "User is not a member of this organization"
                })),
            )
                .into_response()
        }
    }
}

/// Get information about an organization
pub async fn org_info(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(org_name): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Check if the organization exists
    let organizations = state.tenant_state.organizations.read().await;
    let org = match organizations.get(&org_name).cloned() {
        Some(org) => org,
        None => {
            return (StatusCode::NOT_FOUND, "Organization not found").into_response();
        }
    };

    // Check if user is a member of the organization
    if !org.member_ids.contains(&auth_user.user.username) {
        return (
            StatusCode::FORBIDDEN,
            "You are not a member of this organization",
        )
            .into_response();
    }

    // Filter environment variables based on user's role
    let user_role = org
        .member_roles
        .get(&auth_user.user.username)
        .cloned()
        .unwrap_or(crate::tenant::Role::Viewer);

    // Create a filtered copy of the organization with only accessible environment variables
    let mut filtered_org = org.clone();
    filtered_org
        .config
        .shared_environment
        .retain(|_, var| var.min_role <= user_role);

    // Return the organization information
    (StatusCode::OK, Json(filtered_org)).into_response()
}

/// Get a list of all users in an organization
pub async fn org_users_list(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(org_name): Path<String>,
) -> impl IntoResponse {
    // Check if the organization exists
    let organizations = state.tenant_state.organizations.read().await;
    let org = match organizations.get(&org_name) {
        Some(org) => org,
        None => {
            return (StatusCode::NOT_FOUND, "Organization not found").into_response();
        }
    };

    // Check if user is a member of the organization
    if !org.member_ids.contains(&auth_user.user.username) {
        return (
            StatusCode::FORBIDDEN,
            "You are not a member of this organization",
        )
            .into_response();
    }

    // Get all users in the organization
    let users = state.tenant_state.users.read().await;
    let org_users = org
        .member_ids
        .iter()
        .filter_map(|username| users.get(username).cloned())
        .map(|user| {
            let role = org
                .member_roles
                .get(&user.username)
                .cloned()
                .unwrap_or(crate::tenant::Role::Viewer);

            serde_json::json!({
                "username": user.username,
                "name": user.name,
                "role": role,
                "joined_at": user.created_at
            })
        })
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(org_users)).into_response()
}

/// Get a list of all plugins in an organization
pub async fn org_plugins_list(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(org_name): Path<String>,
) -> impl IntoResponse {
    // Check if the organization exists
    let organizations = state.tenant_state.organizations.read().await;
    let org = match organizations.get(&org_name) {
        Some(org) => org,
        None => {
            return (StatusCode::NOT_FOUND, "Organization not found").into_response();
        }
    };

    // Check if user is a member of the organization
    if !org.member_ids.contains(&auth_user.user.username) {
        return (
            StatusCode::FORBIDDEN,
            "You are not a member of this organization",
        )
            .into_response();
    }

    // Get all plugins in the organization
    let org_plugin_refs = state.tenant_state.org_plugin_refs.read().await;
    if let Some(plugins) = org_plugin_refs.get(&org_name) {
        let plugin_registry = state.tenant_state.plugin_registry.read().await;
        let plugins_metadata: Vec<ServerMetadata> = plugins
            .iter()
            .filter_map(|plugin_hash| plugin_registry.get(plugin_hash))
            .filter_map(|worker| Some(ServerMetadata::from(worker.metadata.as_ref()?)))
            .collect();

        (StatusCode::OK, Json(plugins_metadata)).into_response()
    } else {
        (StatusCode::OK, Json(Vec::<serde_json::Value>::new())).into_response()
    }
}

/// Get a list of all pipelines in an organization
pub async fn org_pipelines_list(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(org_name): Path<String>,
) -> impl IntoResponse {
    // Check if the organization exists
    let organizations = state.tenant_state.organizations.read().await;
    let org = match organizations.get(&org_name) {
        Some(org) => org,
        None => {
            return (StatusCode::NOT_FOUND, "Organization not found").into_response();
        }
    };

    // Check if user is a member of the organization
    if !org.member_ids.contains(&auth_user.user.username) {
        return (
            StatusCode::FORBIDDEN,
            "You are not a member of this organization",
        )
            .into_response();
    }

    // Get all pipelines in the organization
    let org_pipelines = state.tenant_state.org_pipelines.read().await;
    if let Some(pipelines) = org_pipelines.get(&org_name) {
        let pipeline_registry = state.tenant_state.pipeline_registry.read().await;
        let pipelines_data = pipelines
            .iter()
            .map(|pipeline_id| {
                if let Some(pipeline) = pipeline_registry.get(pipeline_id) {
                    serde_json::json!({
                        "id": pipeline_id,
                        "pipeline_id": pipeline.pipeline_id,
                        "stages": pipeline.stages.len(),
                        "has_output_constructor": pipeline.output_constructor.is_some()
                    })
                } else {
                    serde_json::json!({
                        "id": pipeline_id,
                        "error": "Pipeline not found in registry"
                    })
                }
            })
            .collect::<Vec<_>>();

        (StatusCode::OK, Json(pipelines_data)).into_response()
    } else {
        (StatusCode::OK, Json(Vec::<serde_json::Value>::new())).into_response()
    }
}

/// Get a list of all daemons in an organization
pub async fn org_daemons_list(
    State(state): State<AppState>,
    auth_user: AuthenticatedUser,
    Path(org_name): Path<String>,
) -> impl IntoResponse {
    // Check if the organization exists
    let organizations = state.tenant_state.organizations.read().await;
    let org = match organizations.get(&org_name) {
        Some(org) => org,
        None => {
            return (StatusCode::NOT_FOUND, "Organization not found").into_response();
        }
    };

    // Check if user is a member of the organization
    if !org.member_ids.contains(&auth_user.user.username) {
        return (
            StatusCode::FORBIDDEN,
            "You are not a member of this organization",
        )
            .into_response();
    }

    // Get all daemons in the organization
    let org_daemons = state.tenant_state.org_daemons.read().await;
    if let Some(daemon_ids) = org_daemons.get(&org_name) {
        let daemon_registry = state.tenant_state.daemon_registry.read().await;
        let daemons_data = daemon_ids
            .iter()
            .filter_map(|daemon_id| {
                daemon_registry.get(daemon_id).map(|daemon| {
                    serde_json::json!({
                        "id": daemon_id,
                        "name": daemon.config.name,
                        "status": daemon.status,
                        "last_run": daemon.last_run,
                        "next_run": daemon.next_run,
                        "frequency_seconds": daemon.config.frequency_seconds
                    })
                })
            })
            .collect::<Vec<_>>();

        (StatusCode::OK, Json(daemons_data)).into_response()
    } else {
        (StatusCode::OK, Json(Vec::<serde_json::Value>::new())).into_response()
    }
}
