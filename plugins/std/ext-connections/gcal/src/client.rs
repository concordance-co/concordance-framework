use crate::log;
use crate::plugin::injector::error::HttpError;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::host::Level;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use waki;
use wstd::runtime::block_on;

/// Calendar operations
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoogleCalendarOperation {
    /// Lists all calendars
    #[default]
    ListCalendars,

    /// Gets a specific calendar
    GetCalendar {
        /// ID of the calendar to retrieve
        calendar_id: String,
    },

    /// Creates a new calendar
    CreateCalendar {
        /// Title of the calendar
        summary: String,
        /// Optional description of the calendar
        description: Option<String>,
        /// Optional geographic location of the calendar
        location: Option<String>,
        /// Optional timezone for the calendar (e.g., "America/Los_Angeles")
        timezone: Option<String>,
    },

    /// Updates a calendar
    UpdateCalendar {
        /// ID of the calendar to update
        calendar_id: String,
        /// New title for the calendar (if provided)
        summary: Option<String>,
        /// New description for the calendar (if provided)
        description: Option<String>,
        /// New location for the calendar (if provided)
        location: Option<String>,
        /// New timezone for the calendar (if provided)
        timezone: Option<String>,
    },

    /// Deletes a calendar
    DeleteCalendar {
        /// ID of the calendar to delete
        calendar_id: String,
    },

    /// Clears all events from a primary calendar
    ClearCalendar {
        /// ID of the calendar to clear
        calendar_id: String,
    },

    /// Gets a specific calendar from the user's calendar list
    GetCalendarListEntry {
        /// ID of the calendar list entry to retrieve
        calendar_id: String,
    },

    /// Adds a calendar to the user's calendar list
    InsertCalendarList {
        /// ID of the calendar to add to the user's list
        calendar_id: String,
    },

    /// Lists events in a calendar
    ListEvents {
        /// ID of the calendar containing the events
        calendar_id: String,
        /// Maximum number of events to return
        max_results: Option<u32>,
        /// Lower bound (inclusive) for an event's start time to filter by
        time_min: Option<String>,
        /// Upper bound (exclusive) for an event's start time to filter by
        time_max: Option<String>,
        /// Free text search terms to find events that match
        q: Option<String>,
        /// Whether to expand recurring events into instances
        single_events: Option<bool>,
        /// Order of the events returned (e.g., "startTime")
        order_by: Option<String>,
    },

    /// Gets a specific event
    GetEvent {
        /// ID of the calendar containing the event
        calendar_id: String,
        /// ID of the event to retrieve
        event_id: String,
    },

    /// Creates a new event
    CreateEvent {
        /// ID of the calendar where the event will be created
        calendar_id: String,
        /// Title of the event
        summary: String,
        /// Geographic location of the event
        location: Option<String>,
        /// Description of the event
        description: Option<String>,
        /// Start time of the event in RFC3339 format
        start_time: String,
        /// End time of the event in RFC3339 format
        end_time: String,
        /// List of attendee email addresses
        attendees: Option<Vec<String>>,
    },

    /// Updates an event
    UpdateEvent {
        /// ID of the calendar containing the event
        calendar_id: String,
        /// ID of the event to update
        event_id: String,
        /// New title for the event (if provided)
        summary: Option<String>,
        /// New location for the event (if provided)
        location: Option<String>,
        /// New description for the event (if provided)
        description: Option<String>,
        /// New start time for the event in RFC3339 format (if provided)
        start_time: Option<String>,
        /// New end time for the event in RFC3339 format (if provided)
        end_time: Option<String>,
        /// New list of attendee email addresses (if provided)
        attendees: Option<Vec<String>>,
    },

    /// Deletes an event
    DeleteEvent {
        /// ID of the calendar containing the event
        calendar_id: String,
        /// ID of the event to delete
        event_id: String,
    },

    /// Creates an event based on a simple text string
    QuickAddEvent {
        /// ID of the calendar where the event will be created
        calendar_id: String,
        /// Text description of the event (e.g., "Dinner with John on Friday 8pm")
        text: String,
    },

    /// Gets instances of a recurring event
    GetEventInstances {
        /// ID of the calendar containing the recurring event
        calendar_id: String,
        /// ID of the recurring event
        event_id: String,
        /// Maximum number of event instances to return
        max_results: Option<u32>,
        /// Lower bound (inclusive) for an event's start time
        time_min: Option<String>,
        /// Upper bound (exclusive) for an event's start time
        time_max: Option<String>,
        /// Token specifying which result page to return
        page_token: Option<String>,
    },

    /// Imports an event from another calendar
    ImportEvent {
        /// ID of the calendar where the event will be imported
        calendar_id: String,
        /// JSON string of the event data to import
        event: String, // JSON string of event data
    },

    /// Moves an event from one calendar to another
    MoveEvent {
        /// ID of the source calendar containing the event
        calendar_id: String,
        /// ID of the event to move
        event_id: String,
        /// ID of the destination calendar
        destination_calendar_id: String,
    },

    /// Queries for free/busy information across calendars
    QueryFreebusy {
        /// Start time for the query in RFC3339 format
        time_min: String,
        /// End time for the query in RFC3339 format
        time_max: String,
        /// List of calendar IDs to query
        calendar_ids: Vec<String>,
        /// Maximum number of calendar IDs to expand from groups
        group_expansion_max: Option<u32>,
        /// Timezone to use for the query
        timezone: Option<String>,
    },
}

pub struct InnerGoogleCalendarClient {
    pub access_token: String,
}

impl InnerGoogleCalendarClient {
    /// Creates a new Google Calendar client with token authentication
    pub fn with_token(token: String) -> Self {
        Self {
            access_token: token,
        }
    }

    /// Executes a Google Calendar operation and returns the result as JSON
    pub fn execute(&self, op: GoogleCalendarOperation) -> Result<Value, PluginError> {
        block_on(async {
            match op {
                GoogleCalendarOperation::ListCalendars => self.list_calendars().await,
                GoogleCalendarOperation::GetCalendar { calendar_id } => {
                    self.get_calendar(&calendar_id).await
                }
                GoogleCalendarOperation::CreateCalendar {
                    summary,
                    description,
                    location,
                    timezone,
                } => {
                    self.create_calendar(summary, description, location, timezone)
                        .await
                }
                GoogleCalendarOperation::UpdateCalendar {
                    calendar_id,
                    summary,
                    description,
                    location,
                    timezone,
                } => {
                    self.update_calendar(&calendar_id, summary, description, location, timezone)
                        .await
                }
                GoogleCalendarOperation::DeleteCalendar { calendar_id } => {
                    self.delete_calendar(&calendar_id).await
                }
                GoogleCalendarOperation::ListEvents {
                    calendar_id,
                    max_results,
                    time_min,
                    time_max,
                    q,
                    single_events,
                    order_by,
                } => {
                    self.list_events(
                        &calendar_id,
                        max_results,
                        time_min,
                        time_max,
                        q,
                        single_events,
                        order_by,
                    )
                    .await
                }
                GoogleCalendarOperation::GetEvent {
                    calendar_id,
                    event_id,
                } => self.get_event(&calendar_id, &event_id).await,
                GoogleCalendarOperation::CreateEvent {
                    calendar_id,
                    summary,
                    location,
                    description,
                    start_time,
                    end_time,
                    attendees,
                } => {
                    self.create_event(
                        &calendar_id,
                        summary,
                        location,
                        description,
                        start_time,
                        end_time,
                        attendees,
                    )
                    .await
                }
                GoogleCalendarOperation::UpdateEvent {
                    calendar_id,
                    event_id,
                    summary,
                    location,
                    description,
                    start_time,
                    end_time,
                    attendees,
                } => {
                    self.update_event(
                        &calendar_id,
                        &event_id,
                        summary,
                        location,
                        description,
                        start_time,
                        end_time,
                        attendees,
                    )
                    .await
                }
                GoogleCalendarOperation::DeleteEvent {
                    calendar_id,
                    event_id,
                } => self.delete_event(&calendar_id, &event_id).await,
                GoogleCalendarOperation::QuickAddEvent { calendar_id, text } => {
                    self.quick_add_event(&calendar_id, &text).await
                }
                GoogleCalendarOperation::ClearCalendar { calendar_id } => {
                    self.clear_calendar(&calendar_id).await
                }

                // New CalendarList operations
                GoogleCalendarOperation::GetCalendarListEntry { calendar_id } => {
                    self.get_calendar_list_entry(&calendar_id).await
                }
                GoogleCalendarOperation::InsertCalendarList { calendar_id } => {
                    self.insert_calendar_list(&calendar_id).await
                }

                // New Event operations
                GoogleCalendarOperation::GetEventInstances {
                    calendar_id,
                    event_id,
                    max_results,
                    time_min,
                    time_max,
                    page_token,
                } => {
                    self.get_event_instances(
                        &calendar_id,
                        &event_id,
                        max_results,
                        time_min,
                        time_max,
                        page_token,
                    )
                    .await
                }
                GoogleCalendarOperation::ImportEvent { calendar_id, event } => {
                    self.import_event(&calendar_id, event).await
                }
                GoogleCalendarOperation::MoveEvent {
                    calendar_id,
                    event_id,
                    destination_calendar_id,
                } => {
                    self.move_event(&calendar_id, &event_id, &destination_calendar_id)
                        .await
                }

                // Freebusy operations
                GoogleCalendarOperation::QueryFreebusy {
                    time_min,
                    time_max,
                    calendar_ids,
                    group_expansion_max,
                    timezone,
                } => {
                    self.query_freebusy(
                        &time_min,
                        &time_max,
                        calendar_ids,
                        group_expansion_max,
                        timezone,
                    )
                    .await
                }
            }
        })
    }

    async fn clear_calendar(&self, calendar_id: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/clear",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );
        let response = self.make_request(waki::Method::Post, &url, None).await?;

        if response.is_empty() {
            Ok(serde_json::json!({ "success": true }))
        } else {
            let json: Value = serde_json::from_slice(&response).map_err(|e| {
                PluginError::Json(format!("Failed to parse clear calendar response: {}", e))
            })?;
            Ok(json)
        }
    }

    // New CalendarList methods
    async fn get_calendar_list_entry(&self, calendar_id: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/users/me/calendarList/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );
        let response = self.make_request(waki::Method::Get, &url, None).await?;

        let json: Value = serde_json::from_slice(&response).map_err(|e| {
            PluginError::Json(format!("Failed to parse calendar list entry: {}", e))
        })?;

        Ok(json)
    }

    async fn insert_calendar_list(&self, calendar_id: &str) -> Result<Value, PluginError> {
        let url = format!("{}/users/me/calendarList", crate::GOOGLE_CALENDAR_BASE_URL);

        let payload = serde_json::to_vec(&serde_json::json!({
            "id": calendar_id
        }))
        .map_err(|e| PluginError::Json(format!("Failed to serialize calendar data: {}", e)))?;

        let response = self
            .make_request(waki::Method::Post, &url, Some(payload))
            .await?;

        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse inserted calendar: {}", e)))?;

        Ok(json)
    }

    // New Event methods
    async fn get_event_instances(
        &self,
        calendar_id: &str,
        event_id: &str,
        max_results: Option<u32>,
        time_min: Option<String>,
        time_max: Option<String>,
        page_token: Option<String>,
    ) -> Result<Value, PluginError> {
        let mut url = format!(
            "{}/calendars/{}/events/{}/instances",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id,
            event_id
        );

        // Build query parameters
        let mut query_params = Vec::new();

        if let Some(max) = max_results {
            query_params.push(format!("maxResults={}", max));
        }

        if let Some(min) = time_min {
            query_params.push(format!("timeMin={}", min));
        }

        if let Some(max) = time_max {
            query_params.push(format!("timeMax={}", max));
        }

        if let Some(token) = page_token {
            query_params.push(format!("pageToken={}", token));
        }

        // Append query parameters to URL
        if !query_params.is_empty() {
            url = format!("{}?{}", url, query_params.join("&"));
        }

        let response = self.make_request(waki::Method::Get, &url, None).await?;

        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse event instances: {}", e)))?;

        Ok(json)
    }

    async fn import_event(&self, calendar_id: &str, event: String) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events/import",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );

        // Parse the event JSON string to ensure it's valid
        let event_json: Value = serde_json::from_str(&event)
            .map_err(|e| PluginError::Json(format!("Invalid event JSON: {}", e)))?;

        let payload = serde_json::to_vec(&event_json)
            .map_err(|e| PluginError::Json(format!("Failed to serialize event data: {}", e)))?;

        let response = self
            .make_request(waki::Method::Post, &url, Some(payload))
            .await?;

        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse imported event: {}", e)))?;

        Ok(json)
    }

    async fn move_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        destination_calendar_id: &str,
    ) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events/{}/move?destination={}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id,
            event_id,
            destination_calendar_id
        );

        let response = self.make_request(waki::Method::Post, &url, None).await?;

        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse moved event: {}", e)))?;

        Ok(json)
    }

    // Freebusy method
    async fn query_freebusy(
        &self,
        time_min: &str,
        time_max: &str,
        calendar_ids: Vec<String>,
        group_expansion_max: Option<u32>,
        timezone: Option<String>,
    ) -> Result<Value, PluginError> {
        let url = format!("{}/freeBusy", crate::GOOGLE_CALENDAR_BASE_URL);

        // Prepare request body
        let mut body = serde_json::json!({
            "timeMin": time_min,
            "timeMax": time_max,
            "items": calendar_ids.iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
        });

        if let Some(max) = group_expansion_max {
            body["groupExpansionMax"] = serde_json::json!(max);
        }

        if let Some(tz) = timezone {
            body["timeZone"] = serde_json::json!(tz);
        }

        let payload = serde_json::to_vec(&body).map_err(|e| {
            PluginError::Json(format!("Failed to serialize freebusy request: {}", e))
        })?;

        let response = self
            .make_request(waki::Method::Post, &url, Some(payload))
            .await?;

        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse freebusy response: {}", e)))?;

        Ok(json)
    }

    async fn list_calendars(&self) -> Result<Value, PluginError> {
        let url = format!("{}/users/me/calendarList", crate::GOOGLE_CALENDAR_BASE_URL);
        let response = self.make_request(waki::Method::Get, &url, None).await?;

        log(
            Level::Info,
            &format!(
                "Calendar list response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse calendar list: {}", e)))?;

        Ok(json)
    }

    async fn get_calendar(&self, calendar_id: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );
        let response = self.make_request(waki::Method::Get, &url, None).await?;

        log(
            Level::Info,
            &format!("Calendar response: {}", String::from_utf8_lossy(&response)),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse calendar: {}", e)))?;

        Ok(json)
    }

    async fn create_calendar(
        &self,
        summary: String,
        description: Option<String>,
        location: Option<String>,
        timezone: Option<String>,
    ) -> Result<Value, PluginError> {
        let url = format!("{}/calendars", crate::GOOGLE_CALENDAR_BASE_URL);

        // Create calendar payload
        let mut calendar = serde_json::json!({
            "summary": summary
        });

        if let Some(desc) = description {
            calendar["description"] = serde_json::json!(desc);
        }

        if let Some(loc) = location {
            calendar["location"] = serde_json::json!(loc);
        }

        if let Some(tz) = timezone {
            calendar["timeZone"] = serde_json::json!(tz);
        }

        let payload = serde_json::to_vec(&calendar)
            .map_err(|e| PluginError::Json(format!("Failed to serialize calendar data: {}", e)))?;

        let response = self
            .make_request(waki::Method::Post, &url, Some(payload))
            .await?;

        log(
            Level::Info,
            &format!(
                "Create calendar response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse created calendar: {}", e)))?;

        Ok(json)
    }

    async fn update_calendar(
        &self,
        calendar_id: &str,
        summary: Option<String>,
        description: Option<String>,
        location: Option<String>,
        timezone: Option<String>,
    ) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );

        // Create update payload with only the fields that are provided
        let mut calendar = serde_json::json!({});

        if let Some(sum) = summary {
            calendar["summary"] = serde_json::json!(sum);
        }

        if let Some(desc) = description {
            calendar["description"] = serde_json::json!(desc);
        }

        if let Some(loc) = location {
            calendar["location"] = serde_json::json!(loc);
        }

        if let Some(tz) = timezone {
            calendar["timeZone"] = serde_json::json!(tz);
        }

        let payload = serde_json::to_vec(&calendar).map_err(|e| {
            PluginError::Json(format!("Failed to serialize calendar update data: {}", e))
        })?;

        let response = self
            .make_request(waki::Method::Put, &url, Some(payload))
            .await?;

        log(
            Level::Info,
            &format!(
                "Update calendar response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse updated calendar: {}", e)))?;

        Ok(json)
    }

    async fn delete_calendar(&self, calendar_id: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );
        let response = self.make_request(waki::Method::Delete, &url, None).await?;

        // If successful, return empty JSON object (delete operations often return no content)
        if response.is_empty() {
            Ok(serde_json::json!({ "success": true }))
        } else {
            let json: Value = serde_json::from_slice(&response).map_err(|e| {
                PluginError::Json(format!("Failed to parse delete response: {}", e))
            })?;
            Ok(json)
        }
    }

    async fn list_events(
        &self,
        calendar_id: &str,
        max_results: Option<u32>,
        time_min: Option<String>,
        time_max: Option<String>,
        q: Option<String>,
        single_events: Option<bool>,
        order_by: Option<String>,
    ) -> Result<Value, PluginError> {
        let mut url = format!(
            "{}/calendars/{}/events",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );

        // Build query parameters
        let mut query_params = Vec::new();

        if let Some(max) = max_results {
            query_params.push(format!("maxResults={}", max));
        }

        if let Some(min) = time_min {
            query_params.push(format!("timeMin={}", min));
        }

        if let Some(max) = time_max {
            query_params.push(format!("timeMax={}", max));
        }

        if let Some(query) = q {
            query_params.push(format!("q={}", query));
        }

        if let Some(single) = single_events {
            query_params.push(format!("singleEvents={}", single));
        }

        if let Some(order) = order_by {
            query_params.push(format!("orderBy={}", order));
        }

        // Append query parameters to URL
        if !query_params.is_empty() {
            url = format!("{}?{}", url, query_params.join("&"));
        }

        let response = self.make_request(waki::Method::Get, &url, None).await?;

        log(
            Level::Info,
            &format!(
                "List events response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse events list: {}", e)))?;

        Ok(json)
    }

    async fn get_event(&self, calendar_id: &str, event_id: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id,
            event_id
        );
        let response = self.make_request(waki::Method::Get, &url, None).await?;

        log(
            Level::Info,
            &format!("Get event response: {}", String::from_utf8_lossy(&response)),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse event: {}", e)))?;

        Ok(json)
    }

    async fn create_event(
        &self,
        calendar_id: &str,
        summary: String,
        location: Option<String>,
        description: Option<String>,
        start_time: String,
        end_time: String,
        attendees: Option<Vec<String>>,
    ) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id
        );

        // Create event payload
        let mut event = serde_json::json!({
            "summary": summary,
            "start": {
                "dateTime": start_time,
                "timeZone": "UTC"
            },
            "end": {
                "dateTime": end_time,
                "timeZone": "UTC"
            }
        });

        if let Some(loc) = location {
            event["location"] = serde_json::json!(loc);
        }

        if let Some(desc) = description {
            event["description"] = serde_json::json!(desc);
        }

        if let Some(att) = attendees {
            let attendee_objects: Vec<serde_json::Value> = att
                .iter()
                .map(|email| serde_json::json!({ "email": email }))
                .collect();
            event["attendees"] = serde_json::json!(attendee_objects);
        }

        let payload = serde_json::to_vec(&event)
            .map_err(|e| PluginError::Json(format!("Failed to serialize event data: {}", e)))?;

        let response = self
            .make_request(waki::Method::Post, &url, Some(payload))
            .await?;

        log(
            Level::Info,
            &format!(
                "Create event response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse created event: {}", e)))?;

        Ok(json)
    }

    async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        summary: Option<String>,
        location: Option<String>,
        description: Option<String>,
        start_time: Option<String>,
        end_time: Option<String>,
        attendees: Option<Vec<String>>,
    ) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id,
            event_id
        );

        // Create update payload with only the fields that are provided
        let mut event = serde_json::json!({});

        if let Some(sum) = summary {
            event["summary"] = serde_json::json!(sum);
        }

        if let Some(loc) = location {
            event["location"] = serde_json::json!(loc);
        }

        if let Some(desc) = description {
            event["description"] = serde_json::json!(desc);
        }

        if let Some(start) = start_time {
            event["start"] = serde_json::json!({
                "dateTime": start,
                "timeZone": "UTC"
            });
        }

        if let Some(end) = end_time {
            event["end"] = serde_json::json!({
                "dateTime": end,
                "timeZone": "UTC"
            });
        }

        if let Some(att) = attendees {
            let attendee_objects: Vec<serde_json::Value> = att
                .iter()
                .map(|email| serde_json::json!({ "email": email }))
                .collect();
            event["attendees"] = serde_json::json!(attendee_objects);
        }

        let payload = serde_json::to_vec(&event).map_err(|e| {
            PluginError::Json(format!("Failed to serialize event update data: {}", e))
        })?;

        let response = self
            .make_request(waki::Method::Put, &url, Some(payload))
            .await?;

        log(
            Level::Info,
            &format!(
                "Update event response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse updated event: {}", e)))?;

        Ok(json)
    }

    async fn delete_event(&self, calendar_id: &str, event_id: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events/{}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id,
            event_id
        );
        let response = self.make_request(waki::Method::Delete, &url, None).await?;

        // If successful, return empty JSON object (delete operations often return no content)
        if response.is_empty() {
            Ok(serde_json::json!({ "success": true }))
        } else {
            let json: Value = serde_json::from_slice(&response).map_err(|e| {
                PluginError::Json(format!("Failed to parse delete response: {}", e))
            })?;
            Ok(json)
        }
    }

    async fn quick_add_event(&self, calendar_id: &str, text: &str) -> Result<Value, PluginError> {
        let url = format!(
            "{}/calendars/{}/events/quickAdd?text={}",
            crate::GOOGLE_CALENDAR_BASE_URL,
            calendar_id,
            url::form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>()
        );

        let response = self.make_request(waki::Method::Post, &url, None).await?;

        log(
            Level::Info,
            &format!(
                "Quick add event response: {}",
                String::from_utf8_lossy(&response)
            ),
        );
        let json: Value = serde_json::from_slice(&response)
            .map_err(|e| PluginError::Json(format!("Failed to parse quick add event: {}", e)))?;

        Ok(json)
    }

    // Helper method to make API requests using waki HTTP client
    async fn make_request(
        &self,
        method: waki::Method,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, PluginError> {
        log(
            Level::Info,
            &format!("Making request: {:?} {}", method, url),
        );

        let mut request_builder = waki::RequestBuilder::new(method, url)
            .header("Authorization", &format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(body_data) = body {
            request_builder = request_builder.body(body_data);
        }

        let response = request_builder.send().map_err(|e| {
            log(Level::Error, &format!("Request error: {}", e));
            PluginError::Http(HttpError::BadStatus(e.to_string()))
        })?;

        let status = response.status_code();
        let body = response.body().map_err(|e| {
            log(Level::Error, &format!("Body error: {}", e));
            PluginError::Http(HttpError::InvalidResponse)
        })?;

        if !(200..300).contains(&status) {
            let error_msg = format!(
                "API error: Status {}, Body: {}",
                status,
                String::from_utf8_lossy(&body)
            );
            log(Level::Error, &error_msg);
            return Err(PluginError::Http(HttpError::BadStatus(error_msg)));
        }

        Ok(body)
    }
}
