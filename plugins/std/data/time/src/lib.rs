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

use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Timelike, Utc, Weekday};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::{inlined_schema_for, with_examples_inlined_schema_for};

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
/// A request to perform a time operation
pub struct TimeRequest {
    /// The operation to perform
    pub operation: TimeOperation,
}

/// The operation to perform
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TimeOperation {
    /// Get the current date and time
    #[default]
    Now,
    /// Parse a date string into a structured format
    Parse {
        /// The date string to parse (e.g. "2023-05-15T14:30:00Z")
        date_string: String,
        /// Optional format string (default: RFC3339/ISO8601)
        format: Option<String>,
    },
    /// Format a date into a specific string format
    Format {
        /// ISO8601 date string to format
        date_string: String,
        /// Format string (e.g. "%Y-%m-%d %H:%M:%S")
        format: String,
    },
    /// Add a duration to a date
    Add {
        /// The base date (ISO8601 format)
        date_string: String,
        /// Number of units to add (negative values subtract)
        value: i64,
        /// Unit of time (seconds, minutes, hours, days, weeks, months, years)
        unit: TimeUnit,
    },
    /// Calculate the difference between two dates
    Difference {
        /// First date (ISO8601 format)
        date1: String,
        /// Second date (ISO8601 format)
        date2: String,
        /// Unit to express the difference in
        unit: TimeUnit,
    },
    /// Check if a date is before another date
    IsBefore {
        /// First date (ISO8601 format)
        date1: String,
        /// Second date (ISO8601 format)
        date2: String,
    },
    /// Check if a date is after another date
    IsAfter {
        /// First date (ISO8601 format)
        date1: String,
        /// Second date (ISO8601 format)
        date2: String,
    },
    /// Convert a timestamp (seconds since epoch) to a date
    FromTimestamp {
        /// Seconds since Unix epoch
        timestamp: i64,
    },
    /// Convert a date to a timestamp (seconds since epoch)
    ToTimestamp {
        /// Date to convert (ISO8601 format)
        date_string: String,
    },
    /// Get the day of week (Monday, Tuesday, etc.) for a date
    DayOfWeek {
        /// Date to analyze (ISO8601 format)
        date_string: String,
    },
    /// Check if a year is a leap year
    IsLeapYear {
        /// Year to check
        year: i32,
    },
    /// Return the start of a time unit (start of day, month, etc.)
    StartOf {
        /// Date to use (ISO8601 format)
        date_string: String,
        /// Unit to get the start of
        unit: TimeUnit,
    },
    /// Return the end of a time unit (end of day, month, etc.)
    EndOf {
        /// Date to use (ISO8601 format)
        date_string: String,
        /// Unit to get the end of
        unit: TimeUnit,
    },
}

/// Time unit for duration operations
#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// Response from the time utility
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TimeResponse {
    /// The result of the operation (could be a date, number, or boolean)
    pub result: serde_json::Value,
    /// Additional information about the result
    pub details: Option<serde_json::Value>,
}

/// Time utility plugin
pub struct TimePlugin;

impl Guest for TimePlugin {
    type JsonToJson = TimeUtil;

    fn get_metadata() -> Metadata {
        Metadata {
            name: "Time Utility".to_string(),
            version: "0.1.0".to_string(),
            author: "Concordance Team".to_string(),
            description: "Utility for working with dates, times, and durations - can provide the current time in UTC".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                TimeRequest,
                TimeRequest::default(),
                TimeRequest { operation: TimeOperation::Now },
                TimeRequest { operation: TimeOperation::Parse { date_string: "2023-05-15T14:30:00Z".to_string(), format: Some("unix".to_string()) } }
            )).unwrap(),
            default_input: serde_json::to_string(&TimeRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(TimeResponse)).unwrap(),
        }
    }
}

pub struct TimeUtil;

impl GuestJsonToJson for TimeUtil {
    fn work(&self, input: String) -> Result<String, PluginError> {
        let request: TimeRequest = serde_json::from_str(&input).map_err(|e| {
            PluginError::Json(format!("Failed to parse input: {} -- input: {}", e, input))
        })?;

        let response = match request.operation {
            TimeOperation::Now => {
                let now = Utc::now();
                TimeResponse {
                    result: serde_json::Value::String(now.to_rfc3339()),
                    details: Some(datetime_to_json(&now)),
                }
            }
            TimeOperation::Parse {
                date_string,
                format,
            } => {
                let dt = parse_date_with_format(&date_string, format.as_deref())?;
                TimeResponse {
                    result: serde_json::Value::String(dt.to_rfc3339()),
                    details: Some(datetime_to_json(&dt)),
                }
            }
            TimeOperation::Format {
                date_string,
                format,
            } => {
                let dt = parse_iso8601(&date_string)?;
                let formatted = format_date(&dt, &format)?;
                TimeResponse {
                    result: serde_json::Value::String(formatted),
                    details: None,
                }
            }
            TimeOperation::Add {
                date_string,
                value,
                unit,
            } => {
                let dt = parse_iso8601(&date_string)?;
                let result = add_duration(&dt, value, &unit)?;
                TimeResponse {
                    result: serde_json::Value::String(result.to_rfc3339()),
                    details: Some(datetime_to_json(&result)),
                }
            }
            TimeOperation::Difference { date1, date2, unit } => {
                let dt1 = parse_iso8601(&date1)?;
                let dt2 = parse_iso8601(&date2)?;
                let (diff, exact_diff) = calculate_difference(&dt1, &dt2, &unit)?;
                TimeResponse {
                    result: serde_json::Value::Number(serde_json::Number::from(diff)),
                    details: Some(serde_json::json!({
                        "exact_difference": exact_diff,
                        "unit": format!("{:?}s", unit),
                        "first_date": dt1.to_rfc3339(),
                        "second_date": dt2.to_rfc3339(),
                    })),
                }
            }
            TimeOperation::IsBefore { date1, date2 } => {
                let dt1 = parse_iso8601(&date1)?;
                let dt2 = parse_iso8601(&date2)?;
                let result = dt1 < dt2;
                TimeResponse {
                    result: serde_json::Value::Bool(result),
                    details: None,
                }
            }
            TimeOperation::IsAfter { date1, date2 } => {
                let dt1 = parse_iso8601(&date1)?;
                let dt2 = parse_iso8601(&date2)?;
                let result = dt1 > dt2;
                TimeResponse {
                    result: serde_json::Value::Bool(result),
                    details: None,
                }
            }
            TimeOperation::FromTimestamp { timestamp } => {
                let dt = from_timestamp(timestamp)?;
                TimeResponse {
                    result: serde_json::Value::String(dt.to_rfc3339()),
                    details: Some(datetime_to_json(&dt)),
                }
            }
            TimeOperation::ToTimestamp { date_string } => {
                let dt = parse_iso8601(&date_string)?;
                let timestamp = dt.timestamp();
                TimeResponse {
                    result: serde_json::Value::Number(serde_json::Number::from(timestamp)),
                    details: Some(serde_json::json!({
                        "date": dt.to_rfc3339(),
                        "milliseconds": timestamp * 1000,
                    })),
                }
            }
            TimeOperation::DayOfWeek { date_string } => {
                let dt = parse_iso8601(&date_string)?;
                let weekday = dt.weekday();
                let (day_num, day_name) = day_of_week(weekday);
                TimeResponse {
                    result: serde_json::Value::String(day_name),
                    details: Some(serde_json::json!({
                        "numeric": day_num,
                        "date": dt.to_rfc3339(),
                        "iso_weekday": weekday.number_from_monday(),
                    })),
                }
            }
            TimeOperation::IsLeapYear { year } => {
                let is_leap = is_leap_year(year);
                TimeResponse {
                    result: serde_json::Value::Bool(is_leap),
                    details: Some(serde_json::json!({
                        "year": year,
                        "days_in_year": if is_leap { 366 } else { 365 },
                    })),
                }
            }
            TimeOperation::StartOf { date_string, unit } => {
                let dt = parse_iso8601(&date_string)?;
                let result = start_of(&dt, &unit)?;
                TimeResponse {
                    result: serde_json::Value::String(result.to_rfc3339()),
                    details: Some(datetime_to_json(&result)),
                }
            }
            TimeOperation::EndOf { date_string, unit } => {
                let dt = parse_iso8601(&date_string)?;
                let result = end_of(&dt, &unit)?;
                TimeResponse {
                    result: serde_json::Value::String(result.to_rfc3339()),
                    details: Some(datetime_to_json(&result)),
                }
            }
        };

        serde_json::to_string(&response)
            .map_err(|e| PluginError::Json(format!("Failed to serialize response: {}", e)))
    }

    fn new() -> Self {
        Self {}
    }
}

// Export the plugin
export!(TimePlugin);

// Time-related utility functions

/// Convert a DateTime to a JSON object with detailed components
fn datetime_to_json<Tz: TimeZone>(dt: &DateTime<Tz>) -> serde_json::Value
where
    Tz::Offset: std::fmt::Display,
{
    let month_name = match dt.month() {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    };

    let (weekday_num, weekday_name) = day_of_week(dt.weekday());

    serde_json::json!({
        "year": dt.year(),
        "month": dt.month(),
        "month_name": month_name,
        "day": dt.day(),
        "hour": dt.hour(),
        "minute": dt.minute(),
        "second": dt.second(),
        "nanosecond": dt.nanosecond(),
        "weekday": weekday_name,
        "weekday_number": weekday_num,
        "timezone": format!("{}", dt.offset()),
        "timestamp": dt.timestamp(),
        "is_leap_year": is_leap_year(dt.year()),
    })
}

/// Parse an ISO8601 date string into a DateTime<Utc>
fn parse_iso8601(date_str: &str) -> Result<DateTime<Utc>, PluginError> {
    DateTime::parse_from_rfc3339(date_str)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            // Try other common formats if RFC3339 fails
            let formats = [
                "%Y-%m-%dT%H:%M:%S%.fZ",
                "%Y-%m-%d %H:%M:%S%.f",
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%d",
            ];

            for format in &formats {
                if let Ok(naive) = NaiveDateTime::parse_from_str(date_str, format) {
                    return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
                }
            }

            Err(PluginError::Generic(format!(
                "Failed to parse date: {}",
                date_str
            )))
        })
}

/// Parse a date string with an optional format
fn parse_date_with_format(
    date_str: &str,
    format: Option<&str>,
) -> Result<DateTime<Utc>, PluginError> {
    match format.map(|format| format.to_lowercase()).as_deref() {
        None => parse_iso8601(date_str),
        Some("unix") | Some("timestamp") => {
            let timestamp = date_str
                .parse::<i64>()
                .map_err(|_| PluginError::Generic("Invalid Unix timestamp".to_string()))?;
            from_timestamp(timestamp)
        }
        Some("iso8601") | Some("rfc3339") => parse_iso8601(date_str),
        Some(format_str) => {
            // Parse with the provided format
            NaiveDateTime::parse_from_str(date_str, format_str)
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
                .map_err(|e| {
                    PluginError::Generic(format!(
                        "Failed to parse date with format {}: {}",
                        format_str, e
                    ))
                })
        }
    }
}

/// Format a date using chrono's format strings
fn format_date<Tz: TimeZone>(dt: &DateTime<Tz>, format: &str) -> Result<String, PluginError>
where
    Tz::Offset: std::fmt::Display,
{
    // Handle some common format aliases
    let format_str = match format {
        "iso8601" | "ISO8601" => "%Y-%m-%dT%H:%M:%S%.fZ",
        "rfc3339" | "RFC3339" => "%Y-%m-%dT%H:%M:%S%:z",
        "YYYY-MM-DD" => "%Y-%m-%d",
        "MM/DD/YYYY" => "%m/%d/%Y",
        "DD/MM/YYYY" => "%d/%m/%Y",
        "YYYY-MM-DD HH:MM:SS" => "%Y-%m-%d %H:%M:%S",
        "HH:MM:SS" => "%H:%M:%S",
        "Month D, YYYY" => "%B %-d, %Y",
        _ => format, // Use the format string as-is
    };

    Ok(dt.format(format_str).to_string())
}

/// Add a duration to a date
fn add_duration(
    dt: &DateTime<Utc>,
    value: i64,
    unit: &TimeUnit,
) -> Result<DateTime<Utc>, PluginError> {
    let duration = match unit {
        TimeUnit::Second => Duration::seconds(value),
        TimeUnit::Minute => Duration::minutes(value),
        TimeUnit::Hour => Duration::hours(value),
        TimeUnit::Day => Duration::days(value),
        TimeUnit::Week => Duration::weeks(value),
        TimeUnit::Month => {
            // Add months by manipulating the date directly
            let month_delta = value.abs() as u32;
            let mut year_delta = month_delta / 12;
            let mut new_month = if value >= 0 {
                dt.month() + (month_delta % 12)
            } else {
                dt.month() - (month_delta % 12)
            };

            // Handle month overflow/underflow
            if value >= 0 && new_month > 12 {
                new_month -= 12;
                year_delta += 1;
            } else if value < 0 && new_month < 1 {
                new_month += 12;
                year_delta += 1;
            }

            let new_year = if value >= 0 {
                dt.year() + year_delta as i32
            } else {
                dt.year() - year_delta as i32
            };

            // Get the number of days in the new month
            let days_in_month = match new_month {
                2 => {
                    if is_leap_year(new_year) {
                        29
                    } else {
                        28
                    }
                }
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };

            // Adjust day if necessary (e.g., Jan 31 + 1 month = Feb 28/29)
            let new_day = std::cmp::min(dt.day(), days_in_month);

            // Create the new datetime
            match Utc.with_ymd_and_hms(
                new_year,
                new_month,
                new_day,
                dt.hour(),
                dt.minute(),
                dt.second(),
            ) {
                chrono::LocalResult::Single(datetime) => return Ok(datetime),
                _ => {
                    return Err(PluginError::Generic(
                        "Invalid date after adding months".to_string(),
                    ))
                }
            }
        }
        TimeUnit::Year => {
            // Add years directly
            let new_year = dt.year() + value as i32;

            // Check if day needs adjustment for leap years (Feb 29 -> Feb 28)
            let new_day = if dt.month() == 2 && dt.day() == 29 && !is_leap_year(new_year) {
                28
            } else {
                dt.day()
            };

            match Utc.with_ymd_and_hms(
                new_year,
                dt.month(),
                new_day,
                dt.hour(),
                dt.minute(),
                dt.second(),
            ) {
                chrono::LocalResult::Single(datetime) => return Ok(datetime),
                _ => {
                    return Err(PluginError::Generic(
                        "Invalid date after adding years".to_string(),
                    ))
                }
            }
        }
    };

    // For most units, we can just add the duration
    if unit != &TimeUnit::Month && unit != &TimeUnit::Year {
        Ok(*dt + duration)
    } else {
        // Month and Year cases are handled above
        unreachable!();
    }
}

/// Calculate the difference between two dates
fn calculate_difference(
    dt1: &DateTime<Utc>,
    dt2: &DateTime<Utc>,
    unit: &TimeUnit,
) -> Result<(i64, f64), PluginError> {
    // Calculate the difference in seconds
    let diff_seconds = dt2.signed_duration_since(*dt1).num_seconds();

    // Convert to the requested unit
    let (diff, exact_diff) = match unit {
        TimeUnit::Second => (diff_seconds, diff_seconds as f64),
        TimeUnit::Minute => (diff_seconds / 60, diff_seconds as f64 / 60.0),
        TimeUnit::Hour => (diff_seconds / 3600, diff_seconds as f64 / 3600.0),
        TimeUnit::Day => (diff_seconds / 86400, diff_seconds as f64 / 86400.0),
        TimeUnit::Week => (diff_seconds / 604800, diff_seconds as f64 / 604800.0),
        TimeUnit::Month => {
            // Approximate months calculation
            let diff_days = diff_seconds / 86400;
            (diff_days / 30, diff_days as f64 / 30.0)
        }
        TimeUnit::Year => {
            // Approximate years calculation
            let diff_days = diff_seconds / 86400;
            (diff_days / 365, diff_days as f64 / 365.0)
        }
    };

    Ok((diff, exact_diff))
}

/// Convert a timestamp to a DateTime<Utc>
fn from_timestamp(timestamp: i64) -> Result<DateTime<Utc>, PluginError> {
    match DateTime::from_timestamp(timestamp, 0) {
        Some(naive) => Ok(naive),
        None => Err(PluginError::Generic(format!(
            "Invalid timestamp: {}",
            timestamp
        ))),
    }
}

/// Get the day of week as (number, name)
fn day_of_week(weekday: Weekday) -> (u8, String) {
    let (num, name) = match weekday {
        Weekday::Mon => (1, "Monday"),
        Weekday::Tue => (2, "Tuesday"),
        Weekday::Wed => (3, "Wednesday"),
        Weekday::Thu => (4, "Thursday"),
        Weekday::Fri => (5, "Friday"),
        Weekday::Sat => (6, "Saturday"),
        Weekday::Sun => (0, "Sunday"),
    };

    (num, name.to_string())
}

/// Check if a year is a leap year
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Calculate the start of a time unit
fn start_of(dt: &DateTime<Utc>, unit: &TimeUnit) -> Result<DateTime<Utc>, PluginError> {
    match unit {
        TimeUnit::Second => Ok(*dt),
        TimeUnit::Minute => {
            match Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 0) {
                chrono::LocalResult::Single(datetime) => Ok(datetime),
                _ => Err(PluginError::Generic(
                    "Invalid date for start of minute".to_string(),
                )),
            }
        }
        TimeUnit::Hour => {
            match Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 0, 0) {
                chrono::LocalResult::Single(datetime) => Ok(datetime),
                _ => Err(PluginError::Generic(
                    "Invalid date for start of hour".to_string(),
                )),
            }
        }
        TimeUnit::Day => match Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 0, 0, 0) {
            chrono::LocalResult::Single(datetime) => Ok(datetime),
            _ => Err(PluginError::Generic(
                "Invalid date for start of day".to_string(),
            )),
        },
        TimeUnit::Week => {
            // Determine days to subtract to get to start of week (Monday)
            let days_to_subtract = match dt.weekday() {
                Weekday::Mon => 0,
                Weekday::Tue => 1,
                Weekday::Wed => 2,
                Weekday::Thu => 3,
                Weekday::Fri => 4,
                Weekday::Sat => 5,
                Weekday::Sun => 6,
            };

            let start_of_week = *dt - Duration::days(days_to_subtract as i64);
            start_of(&start_of_week, &TimeUnit::Day)
        }
        TimeUnit::Month => match Utc.with_ymd_and_hms(dt.year(), dt.month(), 1, 0, 0, 0) {
            chrono::LocalResult::Single(datetime) => Ok(datetime),
            _ => Err(PluginError::Generic(
                "Invalid date for start of month".to_string(),
            )),
        },
        TimeUnit::Year => match Utc.with_ymd_and_hms(dt.year(), 1, 1, 0, 0, 0) {
            chrono::LocalResult::Single(datetime) => Ok(datetime),
            _ => Err(PluginError::Generic(
                "Invalid date for start of year".to_string(),
            )),
        },
    }
}

/// Calculate the end of a time unit
fn end_of(dt: &DateTime<Utc>, unit: &TimeUnit) -> Result<DateTime<Utc>, PluginError> {
    match unit {
        TimeUnit::Second => Ok(*dt),
        TimeUnit::Minute => {
            match Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 59)
            {
                chrono::LocalResult::Single(datetime) => {
                    Ok(datetime.with_nanosecond(999_999_999).unwrap())
                }
                _ => Err(PluginError::Generic(
                    "Invalid date for end of minute".to_string(),
                )),
            }
        }
        TimeUnit::Hour => {
            match Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 59, 59) {
                chrono::LocalResult::Single(datetime) => {
                    Ok(datetime.with_nanosecond(999_999_999).unwrap())
                }
                _ => Err(PluginError::Generic(
                    "Invalid date for end of hour".to_string(),
                )),
            }
        }
        TimeUnit::Day => match Utc.with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 23, 59, 59) {
            chrono::LocalResult::Single(datetime) => {
                Ok(datetime.with_nanosecond(999_999_999).unwrap())
            }
            _ => Err(PluginError::Generic(
                "Invalid date for end of day".to_string(),
            )),
        },
        TimeUnit::Week => {
            // Determine days to add to get to end of week (Sunday)
            let days_to_add = match dt.weekday() {
                Weekday::Mon => 6,
                Weekday::Tue => 5,
                Weekday::Wed => 4,
                Weekday::Thu => 3,
                Weekday::Fri => 2,
                Weekday::Sat => 1,
                Weekday::Sun => 0,
            };

            let end_of_week = *dt + Duration::days(days_to_add as i64);
            end_of(&end_of_week, &TimeUnit::Day)
        }
        TimeUnit::Month => {
            // Calculate the last day of the month
            let days_in_month = match dt.month() {
                2 => {
                    if is_leap_year(dt.year()) {
                        29
                    } else {
                        28
                    }
                }
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };

            match Utc.with_ymd_and_hms(dt.year(), dt.month(), days_in_month, 23, 59, 59) {
                chrono::LocalResult::Single(datetime) => {
                    Ok(datetime.with_nanosecond(999_999_999).unwrap())
                }
                _ => Err(PluginError::Generic(
                    "Invalid date for end of month".to_string(),
                )),
            }
        }
        TimeUnit::Year => match Utc.with_ymd_and_hms(dt.year(), 12, 31, 23, 59, 59) {
            chrono::LocalResult::Single(datetime) => {
                Ok(datetime.with_nanosecond(999_999_999).unwrap())
            }
            _ => Err(PluginError::Generic(
                "Invalid date for end of year".to_string(),
            )),
        },
    }
}
