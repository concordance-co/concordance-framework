use crate::plugin::injector::{
    error::PluginError,
    host::{log, post, HttpRequest, Level},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Add From implementation for serde_json::Error
impl From<serde_json::Error> for PluginError {
    fn from(err: serde_json::Error) -> Self {
        PluginError::Json(err.to_string())
    }
}

pub struct DropboxClient {
    access_token: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub enum DropboxContext {
    Personal {
        /// Team member ID when operating on behalf of a specific team member
        team_member_id: Option<String>,
    },
    #[default]
    Team,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DropboxOperation {
    /// Test operation for connectivity verification
    #[default]
    Test,

    ListTeamMembers {
        /// Maximum number of results to return (default 1000, max 1000)
        limit: Option<u32>,
        /// Whether to include removed members
        include_removed: Option<bool>,
    },

    /// Lists the contents of a folder
    ListFolder {
        context: DropboxContext,
        /// Path to the folder (empty string for root)
        path: String,
        /// Whether to list recursively
        recursive: bool,
        /// Maximum number of results to return (max 2000)
        limit: Option<u32>,
    },

    /// Uploads a file
    UploadFile {
        context: DropboxContext,
        /// Path where the file will be uploaded
        path: String,
        /// File content
        content: String,
        /// Write mode: "add", "overwrite", or "update"
        mode: String,
        /// Whether to auto rename if conflict
        autorename: bool,
    },

    /// Creates a new folder
    CreateFolder {
        context: DropboxContext,
        /// Path for the new folder
        path: String,
        /// Whether to auto rename if conflict
        autorename: bool,
    },

    /// Deletes a file or folder
    Delete {
        context: DropboxContext,
        /// Path to delete
        path: String,
    },

    /// Gets metadata for a file or folder
    GetMetadata {
        context: DropboxContext,
        /// Path to get metadata for
        path: String,
        /// Whether to include media info
        include_media_info: bool,
    },

    /// Moves a file or folder
    Move {
        context: DropboxContext,
        /// Source path
        from_path: String,
        /// Destination path
        to_path: String,
        /// Whether to auto rename if conflict
        autorename: bool,
    },

    /// Copies a file or folder
    Copy {
        context: DropboxContext,
        /// Source path
        from_path: String,
        /// Destination path
        to_path: String,
        /// Whether to auto rename if conflict
        autorename: bool,
    },
}

impl DropboxClient {
    pub fn new(access_token: String) -> Self {
        Self { access_token }
    }

    fn get_headers(
        &self,
        context: &DropboxContext,
        content_type: Option<&str>,
    ) -> Vec<(String, String)> {
        let mut headers = vec![(
            "Authorization".to_string(),
            format!("Bearer {}", self.access_token),
        )];

        if let Some(ct) = content_type {
            headers.push(("Content-Type".to_string(), ct.to_string()));
        }

        // Add team member impersonation header if needed
        if let DropboxContext::Personal {
            team_member_id: Some(id),
        } = context
        {
            headers.push(("Dropbox-API-Select-User".to_string(), id.to_string()));
        }

        headers
    }

    pub fn execute(&self, operation: DropboxOperation) -> Result<String, PluginError> {
        match operation {
            DropboxOperation::Test => {
                log(Level::Info, "Testing Dropbox connection...");
                // Use get_current_account to test connection
                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/users/get_current_account".to_string(),
                    headers: self.get_headers(&DropboxContext::Team, Some("application/json")),
                    body: Vec::new(),
                })?;

                if response.status >= 200 && response.status < 300 {
                    Ok("Connection test successful".to_string())
                } else {
                    Err(PluginError::Unexpected(format!(
                        "Connection test failed with status: {}",
                        response.status
                    )))
                }
            }

            DropboxOperation::ListTeamMembers {
                limit,
                include_removed,
            } => {
                log(Level::Info, "Listing team members");

                let request_body = serde_json::json!({
                    "limit": limit.unwrap_or(1000).min(1000),
                    "include_removed": include_removed.unwrap_or(false)
                });
                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/team/members/list".to_string(),
                    headers: vec![
                        (
                            "Authorization".to_string(),
                            format!("Bearer {}", self.access_token),
                        ),
                        ("Content-Type".to_string(), "application/json".to_string()),
                    ],
                    body: serde_json::to_string(&request_body)?.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }

            DropboxOperation::ListFolder {
                context,
                path,
                recursive,
                limit,
            } => {
                log(Level::Info, &format!("Listing folder: {}", path));

                // Check if we're operating in a team context
                match context {
                    DropboxContext::Team => {
                        // Use the team namespace endpoint for team context
                        // log(
                        //     Level::Info,
                        //     "Using team namespace endpoint for listing folder",
                        // );
                        // let request_body = serde_json::json!({
                        //     "limit": limit.unwrap_or(2000).min(2000)
                        // });
                        // let full_request = HttpRequest {
                        //     url: "https://api.dropboxapi.com/2/team/team_folder/list".to_string(),
                        //     headers: self.get_headers(&context, Some("application/json")),
                        //     body: serde_json::to_string(&request_body)?.into_bytes(),
                        // };
                        // log(Level::Info, &format!("Full request: {:?}", full_request));
                        // let response = post(&full_request)?;

                        // Ok(String::from_utf8_lossy(&response.body).to_string())
                        let request_body = serde_json::json!({
                            "path": path,
                            "recursive": recursive,
                            "include_media_info": false,
                            "include_deleted": false,
                            "include_has_explicit_shared_members": false,
                            "include_mounted_folders": true,
                            "limit": limit.unwrap_or(2000).min(2000)
                        });
                        let full_request = HttpRequest {
                            url: "https://api.dropboxapi.com/2/files/list_folder".to_string(),
                            headers: self.get_headers(&context, Some("application/json")),
                            body: serde_json::to_string(&request_body)?.into_bytes(),
                        };
                        log(Level::Info, &format!("Full request: {:?}", full_request));
                        let response = post(&full_request)?;

                        Ok(String::from_utf8_lossy(&response.body).to_string())
                    }
                    DropboxContext::Personal { .. } => {
                        // Use the regular list_folder endpoint for personal context
                        let request_body = serde_json::json!({
                            "path": path,
                            "recursive": recursive,
                            "include_media_info": false,
                            "include_deleted": false,
                            "include_has_explicit_shared_members": false,
                            "include_mounted_folders": true,
                            "limit": limit.unwrap_or(2000).min(2000)
                        });
                        let full_request = HttpRequest {
                            url: "https://api.dropboxapi.com/2/files/list_folder".to_string(),
                            headers: self.get_headers(&context, Some("application/json")),
                            body: serde_json::to_string(&request_body)?.into_bytes(),
                        };
                        log(Level::Info, &format!("Full request: {:?}", full_request));
                        let response = post(&full_request)?;

                        Ok(String::from_utf8_lossy(&response.body).to_string())
                    }
                }
            }

            DropboxOperation::UploadFile {
                context,
                path,
                content,
                mode,
                autorename,
            } => {
                log(Level::Info, &format!("Uploading file to: {}", path));

                let api_arg = serde_json::json!({
                    "path": path,
                    "mode": mode,
                    "autorename": autorename,
                    "mute": false,
                    "strict_conflict": false
                });

                let response = post(&HttpRequest {
                    url: "https://content.dropboxapi.com/2/files/upload".to_string(),
                    headers: {
                        let mut headers =
                            self.get_headers(&context, Some("application/octet-stream"));
                        headers.push((
                            "Dropbox-API-Arg".to_string(),
                            serde_json::to_string(&api_arg)?,
                        ));
                        headers
                    },
                    body: content.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }

            DropboxOperation::CreateFolder {
                context,
                path,
                autorename,
            } => {
                log(Level::Info, &format!("Creating folder: {}", path));

                let request_body = serde_json::json!({
                    "path": path,
                    "autorename": autorename
                });

                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/files/create_folder_v2".to_string(),
                    headers: self.get_headers(&context, Some("application/json")),
                    body: serde_json::to_string(&request_body)?.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }

            DropboxOperation::Delete { context, path } => {
                log(Level::Info, &format!("Deleting: {}", path));

                let request_body = serde_json::json!({
                    "path": path
                });

                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/files/delete_v2".to_string(),
                    headers: self.get_headers(&context, Some("application/json")),
                    body: serde_json::to_string(&request_body)?.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }

            DropboxOperation::GetMetadata {
                context,
                path,
                include_media_info,
            } => {
                log(Level::Info, &format!("Getting metadata for: {}", path));

                let request_body = serde_json::json!({
                    "path": path,
                    "include_media_info": include_media_info,
                    "include_deleted": false,
                    "include_has_explicit_shared_members": false
                });

                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/files/get_metadata".to_string(),
                    headers: self.get_headers(&context, Some("application/json")),
                    body: serde_json::to_string(&request_body)?.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }

            DropboxOperation::Move {
                context,
                from_path,
                to_path,
                autorename,
            } => {
                log(Level::Info, &format!("Moving {} to {}", from_path, to_path));

                let request_body = serde_json::json!({
                    "from_path": from_path,
                    "to_path": to_path,
                    "allow_shared_folder": false,
                    "autorename": autorename,
                    "allow_ownership_transfer": false
                });

                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/files/move_v2".to_string(),
                    headers: self.get_headers(&context, Some("application/json")),
                    body: serde_json::to_string(&request_body)?.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }

            DropboxOperation::Copy {
                context,
                from_path,
                to_path,
                autorename,
            } => {
                log(
                    Level::Info,
                    &format!("Copying {} to {}", from_path, to_path),
                );

                let request_body = serde_json::json!({
                    "from_path": from_path,
                    "to_path": to_path,
                    "allow_shared_folder": false,
                    "autorename": autorename,
                    "allow_ownership_transfer": false
                });

                let response = post(&HttpRequest {
                    url: "https://api.dropboxapi.com/2/files/copy_v2".to_string(),
                    headers: self.get_headers(&context, Some("application/json")),
                    body: serde_json::to_string(&request_body)?.into_bytes(),
                })?;

                Ok(String::from_utf8_lossy(&response.body).to_string())
            }
        }
    }
}
