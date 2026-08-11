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

use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};
use crate::plugin::injector::logger::Level;
use std::panic;

// host capabilities
use crate::plugin::injector::host::log;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use shared::{inlined_schema_for, with_examples_inlined_schema_for, TryFromEnvVar};
mod client;
use crate::client::*;

const GOOGLE_CALENDAR_BASE_URL: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct GoogleCalendarAuth {
    pub access_token: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct GoogleCalendarRequest {
    #[schemars(skip)]
    pub auth: Option<GoogleCalendarAuth>,
    pub operation: GoogleCalendarOperation,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct GoogleCalendarResponse {
    pub status: String,
    pub data: serde_json::Value,
    pub message: Option<String>,
}

struct GoogleCalendarPlugin;

impl Guest for GoogleCalendarPlugin {
    type JsonToJson = GoogleCalendarClient;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Google Calendar API Client".to_string(),
            version: "0.1.0".to_string(),
            author: "Your Name".to_string(),
            description: "An interface for interacting with the Google Calendar API".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![("auth".to_string(), "GOOGLE_CALENDAR_AUTH".to_string())],
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                GoogleCalendarRequest,
                GoogleCalendarRequest::default(),
                GoogleCalendarRequest {
                    auth: None,
                    operation: GoogleCalendarOperation::ListCalendars
                },
                GoogleCalendarRequest {
                    auth: None,
                    operation: GoogleCalendarOperation::GetCalendar {
                        calendar_id: "primary".to_string()
                    }
                },
                GoogleCalendarRequest {
                    auth: None,
                    operation: GoogleCalendarOperation::CreateEvent {
                        calendar_id: "primary".to_string(),
                        summary: "Team Meeting".to_string(),
                        location: Some("Conference Room 1".to_string()),
                        description: Some("Weekly team sync meeting".to_string()),
                        start_time: "2023-12-25T15:00:00Z".to_string(),
                        end_time: "2023-12-25T16:00:00Z".to_string(),
                        attendees: Some(vec!["colleague@example.com".to_string()])
                    }
                }
            ))
            .unwrap(),
            default_input: serde_json::to_string(&GoogleCalendarRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(GoogleCalendarResponse))
                .unwrap(),
        }
    }
}

pub struct GoogleCalendarClient;

impl GuestJsonToJson for GoogleCalendarClient {
    fn work(&self, input: String) -> Result<String, PluginError> {
        panic::set_hook(Box::new(|err| {
            log(Level::Error, &format!("{}", err));
        }));

        // Parse request
        log(Level::Info, "Got Google Calendar request...");
        let request: GoogleCalendarRequest = serde_json::from_str(&input).map_err(|e| {
            PluginError::Json(format!("Failed to parse Google Calendar request: {}", e))
        })?;

        let auth = match request.auth {
            Some(auth) => auth.clone(),
            None => GoogleCalendarAuth::try_from_env_var("GOOGLE_CALENDAR_AUTH").map_err(|e| {
                PluginError::EnvVar(format!("Failed to load GOOGLE_CALENDAR_AUTH: {}", e))
            })?,
        };

        let client = InnerGoogleCalendarClient::with_token(auth.access_token);
        let result = client.execute(request.operation);

        match result {
            Ok(response) => {
                let response_json = GoogleCalendarResponse {
                    status: "success".to_string(),
                    data: response,
                    message: None,
                };
                serde_json::to_string(&response_json)
                    .map_err(|e| PluginError::Json(format!("Failed to serialize response: {}", e)))
            }
            Err(e) => {
                let error_response = GoogleCalendarResponse {
                    status: "error".to_string(),
                    data: serde_json::json!({}),
                    message: Some(format!("Error: {}", e)),
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

// Helper function to convert serde_json::Error to Plugin Error
impl From<serde_json::Error> for PluginError {
    fn from(err: serde_json::Error) -> Self {
        PluginError::Json(format!("JSON serialization error: {}", err))
    }
}

export!(GoogleCalendarPlugin);
