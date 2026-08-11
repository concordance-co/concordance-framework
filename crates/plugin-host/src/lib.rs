/// Host module for Concordance plugins.
///
/// This module provides the host implementation for plugins,
/// including resource management, HTTP, file conversion, and more.
pub mod host;

/// Injector module that defines interfaces between the host and plugins.
///
/// Contains the component-model based interface definitions that allow
/// plugins to interact with the host application.
pub mod injector;

/// Plugin module for loading and executing WASM plugins.
///
/// Handles loading, compiling, and executing WebAssembly plugins
/// within a sandboxed environment.
pub mod plugin;

/// Server module for the HTTP API.
///
/// Provides the HTTP server implementation, and API endpoints for interacting with the system.
pub mod server;

pub mod daemon;

/// Pipeline module for defining and executing plugin workflows.
///
/// Allows defining sequences of plugin operations that can be
/// executed as a single unit of work.
pub mod pipeline;

pub mod routes;

pub mod tenant;

use serde_json::Value;
use std::borrow::Cow;

mod persistence;

/// Ensures that a JSON path starts with the root selector '$'.
///
/// If the path doesn't start with '$', it prepends it to make the path canonical.
/// This function handles both paths already starting with '$' and those that need it added.
///
/// # Arguments
///
/// * `path` - The JSONPath string to canonicalize
///
/// # Returns
///
/// A `Cow<'_, str>` containing the canonicalized path
fn canonicalize_json_path(path: &str) -> Cow<'_, str> {
    if !path.starts_with('$') {
        format!("${path}").into()
    } else {
        path.into()
    }
}

/// Selects values from a JSON structure using a JSONPath expression.
///
/// This function finds and returns all values in the given JSON that match the provided path.
/// It handles the special case of "." as the root element.
///
/// # Arguments
///
/// * `value` - The JSON value to select from
/// * `path` - The JSONPath expression to use for selection
///
/// # Returns
///
/// * `Ok(Vec<&'a Value>)` - A vector of references to the selected JSON values
/// * `Err(String)` - An error message if selection fails
fn select<'a>(value: &'a Value, mut path: &str) -> Result<Vec<&'a Value>, String> {
    // Handle the special case of the root key
    if path == "." {
        path = "$";
    }
    // format error with debug string because json_path errors may contain newlines
    jsonpath_lib::select(value, &canonicalize_json_path(path))
        .map_err(|e| format!("failed selecting from JSON: {:?}", e.to_string()))
}

/// Replaces values in a JSON structure using a JSONPath expression.
///
/// This function modifies the given JSON by replacing all values that match the provided path
/// with the replacement value.
///
/// # Arguments
///
/// * `value` - The original JSON value to modify
/// * `path` - The JSONPath expression identifying values to replace
/// * `replacement` - The new value to insert at the matched locations
///
/// # Returns
///
/// * `Ok(Value)` - The modified JSON value
/// * `Err(String)` - An error message if replacement fails
fn replace(value: Value, mut path: &str, replacement: Value) -> Result<Value, String> {
    // Handle the special case of the root key
    if path == "." {
        path = "$";
    }

    jsonpath_lib::replace_with(value, &canonicalize_json_path(path), &mut |_| {
        Some(replacement.clone())
    })
    .map_err(|e| format!("failed replacing in JSON: {:?}", e.to_string()))
}
