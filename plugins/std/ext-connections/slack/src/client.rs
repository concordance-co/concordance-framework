// SlackHTTPClient for using with slack-morphism
use crate::plugin::injector::{
    error::PluginError,
    host::{log, post, get, HttpRequest, Level},
};
use base64::engine::{general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures::future::{BoxFuture, FutureExt};
use rvstruct::ValueStruct;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::TryFromEnvVar;
use slack_morphism::{
    errors::*, multipart_form::FileMultipartData, prelude::*, SlackClientId, SlackClientSecret,
};
use url::Url;
use wstd::runtime::block_on;
pub struct WakiSlackConnector;

pub struct SlackExecutor {
    client: SlackClient<WakiSlackConnector>,
    token: SlackApiToken,
}

impl WakiSlackConnector {
    pub fn new() -> Self {
        Self {}
    }
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct SlackUser {
    pub username: String,
    pub icon_url: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SlackOperation {
    /// Gets a list of all users in the workspace. This includes both invited users and deleted/deactivated users.
    #[default]
    UsersList,
    UsersInfo {
        user: String,
    },
    /// Post a message to a channel
    ChatPostMessage {
        /// The channel ID to post to
        channel: String,
        /// The message to post
        text: String,
        /// Provide another message's ts value to make this message a reply. Avoid using a reply's ts value; use its parent instead.
        thread_ts: Option<String>,
        /// A JSON-based array of structured blocks, presented as a URL-encoded string.
        blocks: Option<Vec<serde_json::Value>>,
        /// Whether to post as the authenticated user or as a bot
        as_user: bool,
    },

    /// Get conversation/channel history
    ConversationHistory {
        /// The channel ID
        channel: String,
        /// The max number of messages to return
        limit: Option<u32>,
        /// The latest unix timestamp of messages to return
        latest: Option<String>,
        /// The oldest unix timestamp of messages to return
        oldest: Option<String>,
        /// Whether to include messages from the oldest and/or newest timestamp
        inclusive: Option<bool>,
    },
    /// Get information about a channel
    ChannelInfo {
        /// The channel ID
        channel: String,
    },

    /// List all channels the bot can see
    /// conversations.list
    GetChannels {
        /// Set to true to exclude archived channels from the list.
        exclude_archived: Option<bool>,
        /// Mix and match channel types by providing a comma-separated list of any combination of public_channel, private_channel, mpim, im.
        /// If the response is an error with `missing_scope`, try omitting this field entirely.
        types: Option<String>,
        /// The maximum number of items to return. Default is 100.
        limit: Option<u32>,
        /// Paginate through collections of data by setting the cursor parameter to a next_cursor attribute returned by a previous request's response_metadata
        cursor: Option<String>,
    },

    /// Join a channel. This is useful when the bot is not a member of the channel and needs to be able to read or send messages in it.
    JoinChannel {
        /// The channel ID
        channel: String,
    },
}

impl SlackClientHttpConnector for WakiSlackConnector {
    fn http_get_uri<'a, RS>(
        &'a self,
        full_uri: Url,
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> Deserialize<'de> + Send + 'a,
    {
        async move {
            log(Level::Info, &format!("Slack GET request to {}", full_uri));

            let mut headers = Vec::new();
            if let Some(token) = context.token {
                headers.push((
                    "Authorization".to_string(),
                    format!("Bearer {}", token.token_value.0),
                ));
            }

            let http_req = HttpRequest {
                url: full_uri.to_string(),
                headers,
                body: Vec::new(),
            };

            let response = get(&http_req).map_err(SlackClientError::from)?;

            if response.status >= 200 && response.status < 300 {

                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response.body) {
                    if let Some(ok) = json.get("ok").and_then(|v| v.as_bool()) {
                        if !ok {
                            let error = json
                                .get("error")
                                .and_then(|e| e.as_str())
                                .unwrap_or("Unknown error");
                            return Err(SlackClientError::HttpError(
                                SlackClientHttpError::new(
                                    http::StatusCode::from_u16(response.status)
                                        .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                                )
                                .with_http_response_body(format!("Error: {}", error)),
                            ));
                        }
                    }
                }
                serde_json::from_slice::<RS>(&response.body).map_err(|e| map_serde_error(e, None))
            } else if response.status == 429 {
                // Rate limiting
                let body_str = String::from_utf8_lossy(&response.body);
                Err(SlackClientError::RateLimitError(
                    SlackRateLimitError::new().with_http_response_body(body_str.to_string()),
                ))
            } else {
                let body_str = String::from_utf8_lossy(&response.body);
                Err(SlackClientError::HttpError(
                    SlackClientHttpError::new(
                        http::StatusCode::from_u16(response.status)
                            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                    )
                    .with_http_response_body(body_str.to_string()),
                ))
            }
        }
        .boxed()
    }

    fn http_get_with_client_secret<'a, RS>(
        &'a self,
        full_uri: Url,
        client_id: &'a SlackClientId,
        client_secret: &'a SlackClientSecret,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> Deserialize<'de> + Send + 'a,
    {
        async move {
            log(
                Level::Info,
                &format!("Slack OAuth GET request to {}", full_uri),
            );

            let auth_header = format!(
                "Basic {}",
                BASE64_STANDARD.encode(format!("{}:{}", client_id.value(), client_secret.value()))
            );

            let http_req = HttpRequest {
                url: full_uri.to_string(),
                headers: vec![("Authorization".to_string(), auth_header)],
                body: Vec::new(),
            };

            let response = post(&http_req).map_err(SlackClientError::from)?;

            if response.status >= 200 && response.status < 300 {
                serde_json::from_slice::<RS>(&response.body).map_err(|e| map_serde_error(e, None))
            } else {
                let body_str = String::from_utf8_lossy(&response.body);
                Err(SlackClientError::HttpError(
                    SlackClientHttpError::new(
                        http::StatusCode::from_u16(response.status)
                            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                    )
                    .with_http_response_body(body_str.to_string()),
                ))
            }
        }
        .boxed()
    }

    fn http_post_uri<'a, RQ, RS>(
        &'a self,
        full_uri: Url,
        request_body: &'a RQ,
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RQ: Serialize + Send + Sync,
        RS: for<'de> Deserialize<'de> + Send + 'a,
    {
        async move {
            log(Level::Info, &format!("Slack POST request to {}", full_uri));

            let body = serde_json::to_vec(request_body).map_err(|e| map_serde_error(e, None))?;

            let mut headers = vec![(
                "Content-Type".to_string(),
                "application/json; charset=utf-8".to_string(),
            )];

            if let Some(token) = context.token {
                headers.push((
                    "Authorization".to_string(),
                    format!("Bearer {}", token.token_value.0),
                ));
            }

            let http_req = HttpRequest {
                url: full_uri.to_string(),
                headers,
                body,
            };

            let response = post(&http_req).map_err(SlackClientError::from)?;

            if response.status >= 200 && response.status < 300 {

                // First try to parse as a generic JSON to check for error field
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response.body) {
                    if let Some(ok) = json.get("ok").and_then(|v| v.as_bool()) {
                        if !ok {
                            let error = json
                                .get("error")
                                .and_then(|e| e.as_str())
                                .unwrap_or("Unknown error");
                            return Err(SlackClientError::HttpError(
                                SlackClientHttpError::new(
                                    http::StatusCode::from_u16(response.status)
                                        .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                                )
                                .with_http_response_body(format!("Error: {}", error)),
                            ));
                        }
                    }
                }
                serde_json::from_slice::<RS>(&response.body).map_err(|e| map_serde_error(e, None))
            } else if response.status == 429 {
                // Rate limiting
                let body_str = String::from_utf8_lossy(&response.body);
                Err(SlackClientError::RateLimitError(
                    SlackRateLimitError::new().with_http_response_body(body_str.to_string()),
                ))
            } else {
                let body_str = String::from_utf8_lossy(&response.body);
                Err(SlackClientError::HttpError(
                    SlackClientHttpError::new(
                        http::StatusCode::from_u16(response.status)
                            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                    )
                    .with_http_response_body(body_str.to_string()),
                ))
            }
        }
        .boxed()
    }

    fn http_post_uri_multipart_form<'a, 'p, RS, PT, TS>(
        &'a self,
        full_uri: Url,
        file: Option<FileMultipartData<'p>>,
        params: &'p PT,
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> Deserialize<'de> + Send + 'a,
        PT: IntoIterator<Item = (&'p str, Option<TS>)> + Clone,
        TS: AsRef<str> + 'p + Send,
    {
        log(
            Level::Info,
            &format!("Slack multipart form POST request to {}", full_uri),
        );

        // Create multipart form boundary
        let boundary = format!(
            "----SlackMultipartBoundary{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        let mut form_data = Vec::new();

        // Add params to form
        for (key, value) in params.clone() {
            if let Some(val) = value {
                form_data.extend_from_slice(
                    format!(
                        "--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                        boundary,
                        key,
                        val.as_ref()
                    )
                    .as_bytes(),
                );
            }
        }

        // Add file to form if present
        if let Some(file_data) = file {
            form_data.extend_from_slice(format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    boundary,
                    file_data.name,
                    file_data.name,
                    file_data.content_type
                ).as_bytes());

            form_data.extend_from_slice(file_data.data);
            form_data.extend_from_slice(b"\r\n");
        }

        // Close the form
        form_data.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        let mut headers = vec![(
            "Content-Type".to_string(),
            format!("multipart/form-data; boundary={}", boundary),
        )];

        if let Some(token) = context.token {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", token.token_value.0),
            ));
        }

        let http_req = HttpRequest {
            url: full_uri.to_string(),
            headers,
            body: form_data,
        };

        let response = match post(&http_req) {
            Ok(response) => response,
            Err(e) => return futures::future::err(SlackClientError::from(e)).boxed(),
        };

        if response.status >= 200 && response.status < 300 {
            match serde_json::from_slice::<RS>(&response.body) {
                Ok(result) => futures::future::ok(result).boxed(),
                Err(e) => futures::future::err(map_serde_error(e, None)).boxed(),
            }
        } else {
            let body_str = String::from_utf8_lossy(&response.body);
            futures::future::err(SlackClientError::HttpError(
                SlackClientHttpError::new(
                    http::StatusCode::from_u16(response.status)
                        .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                )
                .with_http_response_body(body_str.to_string()),
            ))
            .boxed()
        }
    }

    fn http_post_uri_binary<'a, 'p, RS>(
        &'a self,
        full_uri: Url,
        content_type: String,
        data: &'a [u8],
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> Deserialize<'de> + Send + 'a,
    {
        async move {
            log(
                Level::Info,
                &format!("Slack binary POST request to {}", full_uri),
            );

            let mut headers = vec![("Content-Type".to_string(), content_type)];

            if let Some(token) = context.token {
                headers.push((
                    "Authorization".to_string(),
                    format!("Bearer {}", token.token_value.0),
                ));
            }

            let http_req = HttpRequest {
                url: full_uri.to_string(),
                headers,
                body: data.to_vec(),
            };

            let response = post(&http_req).map_err(SlackClientError::from)?;

            if response.status >= 200 && response.status < 300 {
                serde_json::from_slice::<RS>(&response.body).map_err(|e| map_serde_error(e, None))
            } else {
                let body_str = String::from_utf8_lossy(&response.body);
                Err(SlackClientError::HttpError(
                    SlackClientHttpError::new(
                        http::StatusCode::from_u16(response.status)
                            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR),
                    )
                    .with_http_response_body(body_str.to_string()),
                ))
            }
        }
        .boxed()
    }
}

impl From<PluginError> for SlackClientError {
    fn from(err: PluginError) -> Self {
        SlackClientError::SystemError(SlackClientSystemError {
            message: Some(format!("{}", err)),
            cause: Some(Box::new(err)),
        })
    }
}

impl SlackExecutor {
    pub fn new(client: SlackClient<WakiSlackConnector>, token: SlackApiToken) -> Self {
        Self { client, token }
    }

    pub fn execute(&self, op: SlackOperation) -> Result<String, PluginError> {
        let session = self.client.open_session(&self.token);
        log(Level::Info, &format!("{:?}", op));
        block_on(async move {
            match op {
                SlackOperation::UsersList => {
                    let result = session.auth_test().await.map_err(|e| {
                        PluginError::Unexpected(format!("Failed to execute auth.test: {}", e))
                    })?;
                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
                SlackOperation::UsersInfo { user } => {
                    let result = session
                        .users_info(&SlackApiUsersInfoRequest::new(SlackUserId::new(user)))
                        .await
                        .map_err(|e| {
                            PluginError::Unexpected(format!("Failed to execute users.info: {}", e))
                        })?;
                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
                SlackOperation::ChatPostMessage {
                    channel,
                    text,
                    thread_ts,
                    blocks,
                    as_user,
                } => {
                    let mut req = SlackApiChatPostMessageRequest::new(
                        SlackChannelId::new(channel),
                        SlackMessageContent::new(),
                    );
                    req.content.text = Some(text);
                    let user = SlackUser::try_from_env_var("SLACK_USER")
                        .map_err(|e| PluginError::Json(format!("No user found: {}", e)))?;
                    if as_user {
                        req.username = Some(user.username);
                        req.icon_url = Some(user.icon_url);
                    }
                    if let Some(blocks_json) = blocks {
                        let blocks: Vec<SlackBlock> = serde_json::from_value(
                            serde_json::Value::Array(blocks_json),
                        )
                        .map_err(|e| PluginError::Json(format!("Invalid blocks format: {}", e)))?;
                        req.content.blocks = Some(blocks);
                    }

                    if let Some(ts) = thread_ts {
                        req.thread_ts = Some(SlackTs::new(ts));
                    }
                    let result = session.chat_post_message(&req).await.map_err(|e| match e {
                        SlackClientError::HttpError(http_err) => PluginError::Unexpected(format!(
                            "Failed to post message: {} - Slack Response: {}",
                            http_err.status_code,
                            http_err.http_response_body.unwrap_or_default()
                        )),
                        _ => PluginError::Unexpected(format!("Failed to post message: {}", e)),
                    })?;

                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
                SlackOperation::ConversationHistory {
                    channel,
                    limit,
                    latest,
                    oldest,
                    inclusive,
                } => {
                    let mut req = SlackApiConversationsHistoryRequest::new();
                    req.channel = Some(SlackChannelId::new(channel));

                    if let Some(lim) = limit {
                        req.limit = Some(lim as u16);
                    }

                    if let Some(latest_ts) = latest {
                        req.latest = Some(SlackTs::new(latest_ts));
                    }

                    if let Some(oldest_ts) = oldest {
                        req.oldest = Some(SlackTs::new(oldest_ts));
                    }

                    if let Some(inc) = inclusive {
                        req.inclusive = Some(inc);
                    }
                    
                    let result = session.conversations_history(&req).await.map_err(|e| {
                        PluginError::Unexpected(format!(
                            "Failed to get conversation history: {}",
                            e
                        ))
                    })?;

                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
                SlackOperation::ChannelInfo { channel } => {
                    let req = SlackApiConversationsInfoRequest::new(SlackChannelId::new(channel));

                    let result = session.conversations_info(&req).await.map_err(|e| {
                        PluginError::Unexpected(format!("Failed to get channel info: {}", e))
                    })?;

                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
                SlackOperation::GetChannels {
                    exclude_archived,
                    types,
                    limit,
                    cursor,
                } => {
                    let mut req = SlackApiConversationsListRequest::new();

                    if let Some(exc) = exclude_archived {
                        req.exclude_archived = Some(exc);
                    }

                    if let Some(type_str) = types {
                        let types_vec: Vec<SlackConversationType> = type_str
                            .split(',')
                            .filter_map(|s| match s.trim() {
                                "public_channel" => Some(SlackConversationType::Public),
                                "private_channel" => Some(SlackConversationType::Private),
                                "mpim" => Some(SlackConversationType::Mpim),
                                "im" => Some(SlackConversationType::Im),
                                _ => None,
                            })
                            .collect();

                        if !types_vec.is_empty() {
                            req.types = Some(types_vec);
                        }
                    }

                    if let Some(lim) = limit {
                        req.limit = Some(lim as u16);
                    }

                    if let Some(curs) = cursor {
                        req.cursor = Some(SlackCursorId::new(curs));
                    }

                    let result = session.conversations_list(&req).await.map_err(|e| {
                        PluginError::Unexpected(format!("Failed to list conversations: {}", e))
                    })?;

                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
                SlackOperation::JoinChannel {
                    channel: channel_id,
                } => {
                    let req =
                        SlackApiConversationsJoinRequest::new(SlackChannelId::new(channel_id));

                    let result = session.conversations_join(&req).await.map_err(|e| {
                        PluginError::Unexpected(format!("Failed to join channel: {}", e))
                    })?;

                    serde_json::to_string(&result).map_err(|e| {
                        PluginError::Json(format!("Failed to serialize response: {}", e))
                    })
                }
            }
        })
    }
}
