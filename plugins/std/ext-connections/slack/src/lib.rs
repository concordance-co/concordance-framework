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
use slack_morphism::prelude::*;
use std::collections::HashMap;
use std::panic;
mod client;
use crate::client::*;

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct SlackAuth {
    pub token: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
/// We already have Auth handled via environment variables, so we don't need to pass it in the request.
pub struct SlackRequest {
    #[schemars(skip)]
    pub auth: Option<SlackAuth>,
    /// The slack operation to perform. If the selected operation requires a channelId, its the unique identifier NOT the name of the channel.
    /// It may be necessary to first call `GetChannels` to convert a channel name to its unique identifier.
    pub operation: SlackOperation,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct SlackResponse {
    pub status: String,
    pub data: serde_json::Value,
    pub message: Option<String>,
}

// Main plugin struct
pub struct SlackPlugin;

impl Guest for SlackPlugin {
    type JsonToJson = SlackHandler;

    fn get_metadata() -> Metadata {
        log(
            Level::Info,
            &format!(
                "Input Schema: {}",
                serde_json::to_string_pretty(&with_examples_inlined_schema_for!(
                    SlackRequest,
                    SlackRequest::default(),
                    SlackRequest {
                        auth: None,
                        operation: SlackOperation::ConversationHistory {
                            channel: "C08JBUZGY4T".to_string(),
                            limit: Some(10),
                            latest: None,
                            oldest: None,
                            inclusive: None
                        }
                    }
                ))
                .unwrap()
            ),
        );

        Metadata {
            name: "Slack Integration".to_string(),
            version: "0.1.0".to_string(),
            author: "Marshall Vyletel".to_string(),
            description: "Direct Slack API integration for reading and writing messages"
                .to_string(),
            env_var_support: vec![("token".to_string(), "SLACK_AUTH".to_string())],
            kind: PluginKind::Tool,
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                SlackRequest,
                SlackRequest::default(),
                SlackRequest {
                    auth: None,
                    operation: SlackOperation::ConversationHistory {
                        channel: "C08JBUZGY4T".to_string(),
                        limit: Some(10),
                        latest: None,
                        oldest: None,
                        inclusive: None
                    }
                },
                SlackRequest {
                    auth: None,
                    operation: SlackOperation::ChatPostMessage {
                        channel: "C08HHEMNXNU".to_string(),
                        text: "Hello from Concordance EA!".to_string(),
                        thread_ts: None,
                        blocks: None,
                        as_user: false,
                    }
                }
            ))
            .unwrap(),
            default_input: serde_json::to_string(&SlackRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(SlackResponse)).unwrap(),
        }
    }
}

pub struct SlackHandler;

impl GuestJsonToJson for SlackHandler {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Implement the logic here
        panic::set_hook(Box::new(|err| {
            log(Level::Error, &format!("{}", err));
        }));

        let request: SlackRequest = serde_json::from_str(&input)
            .map_err(|e| PluginError::Json(format!("Failed to parse Slack request: {}", e)))?;

        let auth = match request.auth {
            Some(auth) => auth.clone(),
            None => SlackAuth::try_from_env_var("SLACK_AUTH")
                .map_err(|e| PluginError::EnvVar(format!("Failed to load SLACK_AUTH: {}", e)))?,
        };
        let client = SlackClient::new(WakiSlackConnector::new());
        let token_value: SlackApiTokenValue = auth.token.into();
        let token = SlackApiToken::new(token_value);
        let operation = request.operation;
        let executor = SlackExecutor::new(client, token);
        log(Level::Info, "Executing Slack operation...");
        let response = executor.execute(operation).map_err(|e| {
            PluginError::Unexpected(format!("Failed to execute Slack operation: {}", e))
        })?;
        let mut json: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse response: {}", e)))?;

        let mut cache_map: HashMap<String, String> = HashMap::new();
        swap_userid(&mut json, &mut cache_map, &executor)?;

        // Return the response
        serde_json::to_string(&json)
            .map_err(|e| PluginError::Json(format!("Failed to serialize response: {}", e)))
    }

    fn new() -> Self {
        Self {}
    }
}

fn swap_userid(
    json: &mut serde_json::Value,
    cache_map: &mut HashMap<String, String>,
    executor: &SlackExecutor,
) -> Result<(), PluginError> {
    match json {
        serde_json::Value::Object(map) => {
            // Check for user field that might contain a user ID
            if let Some(user) = map.get("user") {
                if let Some(user_id) = user.as_str() {
                    // Check if we have a cached mapping first

                    // Check if we have a cached mapping first
                    if let Some(username) = cache_map.get(user_id) {
                        map.insert(
                            "user".to_string(),
                            serde_json::Value::String(username.to_string()),
                        );
                    } else {
                        // Fallback to using UsersInfo operation if not in user object
                        let user_info_op = SlackOperation::UsersInfo {
                            user: user_id.to_string(),
                        };

                        let user_info = executor.execute(user_info_op).map_err(|e| {
                            PluginError::Unexpected(format!("Failed to fetch user info: {}", e))
                        })?;
                        log(Level::Info, &format!("found user: {}", &user_info));
                        let user_info: serde_json::Value = serde_json::from_str(&user_info)
                            .map_err(|e| {
                                PluginError::Json(format!("Failed to parse user info: {}", e))
                            })?;
                        if let Some(name) = user_info
                            .get("user")
                            .and_then(|u| u.get("name"))
                            .and_then(|n| n.as_str())
                        {
                            cache_map.insert(user_id.to_string(), name.to_string());
                            map.insert(
                                "user".to_string(),
                                serde_json::Value::String(name.to_string()),
                            );
                        }
                    }
                }
            }
            // Recursively process all values in the object
            for (_, value) in map.iter_mut() {
                swap_userid(value, cache_map, executor)?;
            }
        }
        serde_json::Value::Array(arr) => {
            for value in arr.iter_mut() {
                swap_userid(value, cache_map, executor)?;
            }
        }
        _ => {}
    }
    Ok(())
}

export!(SlackPlugin);
