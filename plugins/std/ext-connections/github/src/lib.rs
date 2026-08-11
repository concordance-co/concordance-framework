// Construct the injector plugin interface
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

use crate::plugin::injector::logger::Level;

use std::panic;

use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};

// host capabilities
use crate::plugin::injector::host::log;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use shared::{inlined_schema_for, with_examples_inlined_schema_for, TryFromEnvVar};
mod client;
use crate::client::*;
// mod http_client;
// use crate::http_client::WakiClient;

const GITHUB_BASE_URL: &str = "https://api.github.com";
// const GITHUB_BASE_UPLOAD_URI: &str = "https://uploads.github.com";

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct GitHubAuth {
    pub token: String,
    pub username: Option<String>,
    pub base_uri: Option<String>,
    pub upload_uri: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct GitHubRequest {
    #[schemars(skip)]
    pub auth: Option<GitHubAuth>,
    pub operation: GitHubOperation,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct GitHubResponse {
    pub status: String,
    pub data: serde_json::Value,
    pub message: Option<String>,
}

struct GitHubPlugin;

impl Guest for GitHubPlugin {
    type JsonToJson = GitHubClient;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "GitHub API Client".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "An interface for interacting with the GitHub API using Octocrab"
                .to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![("auth".to_string(), "GITHUB_AUTH".to_string())],
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                GitHubRequest,
                GitHubRequest::default(),
                GitHubRequest {
                    auth: None,
                    operation: GitHubOperation::ListUserRepos {
                        username: "test_user_name".to_string(),
                        per_page: Some(10),
                        page: Some(2)
                    }
                },
                GitHubRequest {
                    auth: None,
                    operation: GitHubOperation::CreatePull {
                        owner: "test_user_name".to_string(),
                        repo: "test_repo_name".to_string(),
                        title: "Pull Request Title".to_string(),
                        body: Some("Implementation details".to_string()),
                        head: "64afd131".to_string(),
                        base: "main".to_string(),
                        draft: Some(true),
                    }
                },
                GitHubRequest {
                    auth: None,
                    operation: GitHubOperation::CreateIssue {
                        owner: "test_user_name".to_string(),
                        repo: "test_repo_name".to_string(),
                        title: "Issue Creation Title".to_string(),
                        body: Some("Description of the issue".to_string()),
                        labels: Some(vec!["bug".to_string()]),
                        assignees: None,
                    }
                }
            ))
            .unwrap(),
            default_input: serde_json::to_string(&GitHubRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(GitHubResponse)).unwrap(),
        }
    }
}

pub struct GitHubClient;

impl GuestJsonToJson for GitHubClient {
    fn work(&self, input: String) -> Result<String, PluginError> {
        panic::set_hook(Box::new(|err| {
            log(Level::Error, &format!("{}", err));
        }));
        // Parse request
        log(Level::Info, "Got GitHub request...");
        let request: GitHubRequest = serde_json::from_str(&input)
            .map_err(|e| PluginError::Json(format!("Failed to parse GitHub request: {}", e)))?;

        let auth = match request.auth {
            Some(auth) => auth.clone(),
            None => GitHubAuth::try_from_env_var("GITHUB_AUTH")
                .map_err(|e| PluginError::EnvVar(format!("Failed to load GITHUB_AUTH: {}", e)))?,
        };

        let client = InnerGitHubClient::with_token(auth.token);
        let response = client.execute(request.operation).map_err(|e| {
            PluginError::Unexpected(format!("Failed to execute GitHub request: {}", e))
        })?;

        serde_json::to_string(&response)
            .map_err(|e| PluginError::Json(format!("Failed to serialize response: {}", e)))
    }

    fn new() -> Self {
        Self {}
    }
}

// Helper function to convert serde_json::Value to Plugin Error
impl From<serde_json::Error> for PluginError {
    fn from(err: serde_json::Error) -> Self {
        PluginError::Json(format!("JSON serialization error: {}", err))
    }
}

export!(GitHubPlugin);
