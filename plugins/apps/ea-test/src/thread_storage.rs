use crate::exports::plugin::injector::guest::PluginError;
use crate::plugin::injector::error::FsError;
use crate::plugin::injector::host::log;
use crate::plugin::injector::logger::Level;
use crate::plugin::injector::open_a_i_like::ChatSession;
use crate::types::*;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn save_thread_data(
    chat_session: &ChatSession,
    thread_id: &str,
    title: Option<String>,
) -> Result<(), PluginError> {
    let threads_dir = "threads";
    let thread_file_path = format!("{}/thread-{}.json", threads_dir, thread_id);

    // Get updated title if available
    let title = title.or_else(|| chat_session.session_title().map(|t| t.to_string()));

    // Get all messages from the session
    let messages = chat_session.messages();

    let config = chat_session.config();

    // Store the thread data
    let thread_storage = ThreadStorage {
        title: title.clone(),
        messages,
        config: Some(config),
    };

    let thread_json = serde_json::to_string(&thread_storage)
        .map_err(|e| PluginError::Json(format!("Failed to serialize thread data: {}", e)))?;

    fs::write(&thread_file_path, thread_json).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to write thread file: {}",
            e
        )))
    })?;

    log(
        Level::Info,
        &format!("Saved thread to {}", thread_file_path),
    );

    // Update threads summary
    update_thread_summary(thread_id, title)?;

    Ok(())
}

fn update_thread_summary(thread_id: &str, title: Option<String>) -> Result<(), PluginError> {
    let threads_dir = "threads";
    let summary_path = format!("{}/threads_summary.json", threads_dir);
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Create or update the thread summary
    let mut thread_summary = if Path::new(&summary_path).exists() {
        let summary_data = fs::read_to_string(&summary_path).map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to read summary file: {}",
                e
            )))
        })?;

        serde_json::from_str::<ThreadSummary>(&summary_data).unwrap_or_default()
    } else {
        ThreadSummary::default()
    };

    // Update or add this thread's information
    if let Some(existing_thread) = thread_summary
        .threads
        .iter_mut()
        .find(|t| t.id == thread_id)
    {
        existing_thread.title = title;
        existing_thread.last_message_timestamp = current_time;
    } else {
        thread_summary.threads.push(ThreadInfo {
            id: thread_id.to_string(),
            title,
            last_message_timestamp: current_time,
        });
    }

    // Write the updated summary back to file
    let summary_json = serde_json::to_string(&thread_summary)
        .map_err(|e| PluginError::Json(format!("Failed to serialize thread summary: {}", e)))?;

    fs::write(&summary_path, summary_json).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to write thread summary file: {}",
            e
        )))
    })?;

    log(
        Level::Info,
        &format!("Updated threads summary at {}", summary_path),
    );

    Ok(())
}
