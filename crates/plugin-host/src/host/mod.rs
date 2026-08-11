//! Host module for Concordance plugins.
//!
//! This module provides the host implementation for Concordance plugins,
//! including resource management, HTTP, markdown conversion, and more.

/// Core host implementation for plugin functionality.
mod host_impl;
/// WASI context wrapper for the host.
mod host_wrapper;
/// Service provider implementations for various plugin functionalities.
mod providers;

pub use host_impl::Host;
pub use host_wrapper::HostHolder;
pub use providers::*;

use std::path::{Path, PathBuf};

/// Returns the writeable path for Concordance application data.
///
/// This function determines the appropriate directory for Concordance to store its data:
/// - First tries to use the user's home directory (~/.concordance)
/// - Falls back to the current working directory if home directory is unavailable
///
/// The function ensures the directory exists by creating it if necessary.
///
/// # Returns
/// A `PathBuf` pointing to the writeable directory (typically ~/.concordance)
///
/// # Panics
/// Panics if the directory cannot be created due to filesystem permissions or other I/O errors
pub fn writeable_path() -> PathBuf {
    let home_dir = match home::home_dir() {
        Some(path) if !path.as_os_str().is_empty() => path,
        _ => std::env::current_dir().expect("Failed to get current directory"),
    };
    let concordance_dir = home_dir.join(".concordance");
    if !concordance_dir.exists() {
        std::fs::create_dir_all(&concordance_dir)
            .expect("Failed to create ~/.concordance directory");
    }
    concordance_dir
}

/// Normalizes a relative path by resolving directory navigation components.
///
/// This function processes a path and handles special path components:
/// - `..` navigates up one directory level (removes the last component)
/// - `.` is ignored (current directory)
/// - Normal path components are preserved
/// - Root/prefix components are preserved as-is
///
/// # Parameters
/// - `path`: The source path to normalize
///
/// # Returns
/// A new `PathBuf` with all directory navigations resolved
///
/// # Examples
/// ```
/// let path = Path::new("dir1/dir2/../dir3/./dir4");
/// let normalized = normalize_relative_path(path);
/// // normalized will be "dir1/dir3/dir4"
/// ```
pub fn normalize_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Go up one directory by popping the last component
                normalized.pop();
            }
            std::path::Component::CurDir => {
                // Ignore "./"
                continue;
            }
            std::path::Component::Normal(part) => {
                // Add normal path components
                normalized.push(part);
            }
            _ => {
                // Handle root or prefix if needed (e.g., "/" or "C:\")
                normalized.push(component.as_os_str());
            }
        }
    }

    normalized
}
