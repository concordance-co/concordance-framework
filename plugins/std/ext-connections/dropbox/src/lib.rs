wit_bindgen::generate!({
    world: "injector",
    path: "../../../../wit",
    additional_derives: [
        serde::Serialize,
        serde::Deserialize,
        Clone,
        PartialEq,
    ],
});

use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};
use crate::plugin::injector::{host::log, logger::Level};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::{inlined_schema_for, with_examples_inlined_schema_for, TryFromEnvVar};
use std::panic;

mod client;
use client::{DropboxClient, DropboxContext, DropboxOperation};

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct DropboxAuth {
    pub access_token: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct DropboxRequest {
    #[schemars(skip)]
    pub auth: Option<DropboxAuth>,
    /// The Dropbox operation to perform
    pub operation: DropboxOperation,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct DropboxResponse {
    pub status: String,
    pub data: serde_json::Value,
    pub message: Option<String>,
}

pub struct DropboxPlugin;

impl Guest for DropboxPlugin {
    type JsonToJson = DropboxHandler;

    fn get_metadata() -> Metadata {
        log(
            Level::Info,
            &format!(
                "Dropbox Plugin - Input Schema: {}",
                serde_json::to_string_pretty(&with_examples_inlined_schema_for!(
                    DropboxRequest,
                    DropboxRequest::default(),
                    DropboxRequest {
                        auth: None,
                        operation: DropboxOperation::ListFolder {
                            path: "".to_string(),
                            recursive: false,
                            limit: Some(100),
                            context: DropboxContext::Team,
                        }
                    }
                ))
                .unwrap()
            ),
        );

        Metadata {
            name: "Dropbox Integration".to_string(),
            version: "0.2.0".to_string(),
            author: "Concordance Team".to_string(),
            description: "Dropbox API integration for basic file and folder CRUD operations. Supports both personal and team contexts.".to_string(),
            env_var_support: vec![(
                "access_token".to_string(),
                "DROPBOX_ACCESS_TOKEN".to_string(),
            )],
            kind: PluginKind::Tool,
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                DropboxRequest,
                // Default example
                DropboxRequest::default(),
                // List folder example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::ListFolder {
                        path: "/documents".to_string(),
                        recursive: false,
                        limit: Some(100),
                        context: DropboxContext::Team,
                    }
                },
                // List team members example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::ListTeamMembers {
                        limit: Some(100),
                        include_removed: Some(false),
                    }
                },
                // Upload file example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::UploadFile {
                        path: "/documents/new_file.txt".to_string(),
                        content: "Hello, Dropbox!".to_string(),
                        mode: "add".to_string(),
                        autorename: true,
                        context: DropboxContext::Team,
                    }
                },
                // Create folder example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::CreateFolder {
                        path: "/projects/new_project".to_string(),
                        autorename: true,
                        context: DropboxContext::Team,
                    }
                },
                // Team context example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::ListFolder {
                        path: "/team_documents".to_string(),
                        recursive: false,
                        limit: Some(50),
                        context: DropboxContext::Personal {
                            team_member_id: Some("dbmid:AABBCCDDEEFFGGHHIIjj".to_string())
                        },
                    }
                },
                // Delete example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::Delete {
                        path: "/old_file.txt".to_string(),
                        context: DropboxContext::Team,
                    }
                },
                // Move example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::Move {
                        from_path: "/documents/old_location.txt".to_string(),
                        to_path: "/archive/new_location.txt".to_string(),
                        autorename: false,
                        context: DropboxContext::Team,
                    }
                },
                // Copy example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::Copy {
                        from_path: "/templates/template.docx".to_string(),
                        to_path: "/documents/new_document.docx".to_string(),
                        autorename: true,
                        context: DropboxContext::Team,
                    }
                },
                // Get metadata example
                DropboxRequest {
                    auth: None,
                    operation: DropboxOperation::GetMetadata {
                        path: "/photos/vacation.jpg".to_string(),
                        include_media_info: true,
                        context: DropboxContext::Team,
                    }
                }
            ))
            .unwrap(),
            default_input: serde_json::to_string(&DropboxRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(DropboxResponse)).unwrap(),
        }
    }
}

pub struct DropboxHandler;

impl GuestJsonToJson for DropboxHandler {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Set up panic hook to log errors
        panic::set_hook(Box::new(|err| {
            log(Level::Error, &format!("Panic occurred: {}", err));
        }));
        log(Level::Info, "Attempting to not kill myself");
        // Parse the request
        let request: DropboxRequest = serde_json::from_str(&input)
            .map_err(|e| PluginError::Json(format!("Failed to parse Dropbox request: {}", e)))?;

        // Get authentication - either from request or environment variable
        let auth = match request.auth {
            Some(auth) => auth.clone(),
            None => DropboxAuth::try_from_env_var("DROPBOX_ACCESS_TOKEN").map_err(|e| {
                PluginError::EnvVar(format!("Failed to load DROPBOX_ACCESS_TOKEN: {}", e))
            })?,
        };

        // Create client with access token
        let client = DropboxClient::new(auth.access_token);

        // Log the operation being performed
        let operation_name = match &request.operation {
            DropboxOperation::Test => "Test",
            DropboxOperation::ListTeamMembers { .. } => "ListTeamMembers",
            DropboxOperation::ListFolder { .. } => "ListFolder",
            DropboxOperation::UploadFile { .. } => "UploadFile",
            DropboxOperation::CreateFolder { .. } => "CreateFolder",
            DropboxOperation::Delete { .. } => "Delete",
            DropboxOperation::GetMetadata { .. } => "GetMetadata",
            DropboxOperation::Move { .. } => "Move",
            DropboxOperation::Copy { .. } => "Copy",
        };
        log(
            Level::Info,
            &format!("Executing Dropbox operation: {}", operation_name),
        );

        // Execute the operation
        match client.execute(request.operation) {
            Ok(response) => {
                // Try to parse the response as JSON
                let response_json: serde_json::Value = match serde_json::from_str(&response) {
                    Ok(json) => json,
                    Err(_) => {
                        // If it's not valid JSON (like raw file content), wrap it
                        serde_json::json!({
                            "raw_content": response
                        })
                    }
                };

                let dropbox_response = DropboxResponse {
                    status: "success".to_string(),
                    data: response_json,
                    message: None,
                };

                serde_json::to_string(&dropbox_response)
                    .map_err(|e| PluginError::Json(format!("Failed to serialize response: {}", e)))
            }
            Err(e) => {
                log(Level::Error, &format!("Dropbox operation failed: {}", e));

                let error_response = DropboxResponse {
                    status: "error".to_string(),
                    data: serde_json::Value::Null,
                    message: Some(format!("Operation failed: {}", e)),
                };

                serde_json::to_string(&error_response).map_err(|e| {
                    PluginError::Json(format!("Failed to serialize error response: {}", e))
                })
            }
        }
    }

    fn new() -> Self {
        Self {}
    }
}

export!(DropboxPlugin);
