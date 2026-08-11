//! This module implements a Sandbox Reader plugin for file system interactions
//! within a sandboxed environment.

// Construct the injector plugin interface
wit_bindgen::generate!({
    world: "injector",
    path: "../../../../wit",
    generate_all,
});

use base64::prelude::*;
use exports::plugin::injector::guest::{Guest, GuestJsonToJson, Metadata, PluginError, PluginKind};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::{inlined_schema_for, with_examples_inlined_schema_for};

/// Represents the actions that can be performed by the Sandbox Reader.
///
/// # Variants
///
/// * `dirFiles` - List all files in the specified directory
/// * `fileRead` - Read the content of a specific file
/// * `fileWrite` - Write content to a file
/// * `fileDelete` - Delete a specific file
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Action {
    /// List all files in a directory
    #[serde(
        alias = "dirFiles",
        alias = "dir_files",
        alias = "dir-files",
        alias = "dir files",
        alias = "DirFiles"
    )]
    DirFiles { dir: String },

    /// Read a specific file's content
    #[serde(
        alias = "fileRead",
        alias = "file_read",
        alias = "file-read",
        alias = "file read",
        alias = "FileRead"
    )]
    FileRead {
        #[serde(
            alias = "path",
            alias = "filePath",
            alias = "file_path",
            alias = "filepath"
        )]
        /// file: The path to the file to read
        file: String,
        #[serde(
            alias = "asString",
            alias = "as_string",
            alias = "as-string",
            alias = "as string",
            alias = "AsString"
        )]
        /// asString: Whether to return the file content as a string instead of base64
        as_string: bool,
    },

    /// Write content to the file. If append is not set to true, it will overwrite the file with the new content.
    #[serde(
        alias = "fileWrite",
        alias = "file_write",
        alias = "file-write",
        alias = "file write",
        alias = "FileWrite"
    )]
    FileWrite {
        #[serde(
            alias = "path",
            alias = "filePath",
            alias = "file_path",
            alias = "filepath"
        )]
        /// file: The path to the file to write
        file: String,
        /// It is already a string, just write the raw string instead of base64 decoding
        write_raw: bool,
        /// append: whether to append to the file instead of overwriting it
        append: bool,
        /// content: Base64-encoded or raw string content to write to the file
        content: String,
    },

    /// Delete a file
    #[serde(
        alias = "fileDelete",
        alias = "file_delete",
        alias = "file-delete",
        alias = "file delete",
        alias = "FileDelete"
    )]
    FileDelete {
        #[serde(
            alias = "path",
            alias = "filePath",
            alias = "file_path",
            alias = "filepath"
        )]
        /// file: The path to the file to delete
        file: String,
    },
}

impl Default for Action {
    /// Default action is to list files in the current directory
    fn default() -> Self {
        Action::DirFiles {
            dir: ".".to_string(),
        }
    }
}

/// Request structure for the Sandbox Reader plugin
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ReaderRequest {
    // #[serde(flatten)]
    /// The action to perform (dirFiles, fileRead, fileWrite, or fileDelete)
    pub action: Action,
}

/// Response structure from the Sandbox Reader plugin
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Resp {
    /// The result of the operation:
    /// - For fileRead: base64-encoded or string file content
    /// - For dirFiles: JSON string of file paths
    /// - For fileWrite: success message
    /// - For fileDelete: success message
    pub result: String,
}

/// Implementation of the Sandbox Reader plugin
struct SandboxReaderPlugin;

impl Guest for SandboxReaderPlugin {
    type JsonToJson = SandboxReader;

    /// Returns metadata about the Sandbox Reader plugin
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Sandbox FS".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "Reads and writes files in the sandbox".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                ReaderRequest,
                ReaderRequest::default(),
                ReaderRequest {
                    action: Action::FileRead {
                        file: "./memories.md".to_string(),
                        as_string: true
                    }
                },
                ReaderRequest {
                    action: Action::FileWrite {
                        file: "./memories.md".to_string(),
                        content: "".to_string(),
                        write_raw: true,
                        append: true,
                    }
                },
                ReaderRequest {
                    action: Action::FileDelete {
                        file: "./unwanted_file.txt".to_string(),
                    }
                }
            ))
            .unwrap(),
            default_input: serde_json::to_string(&ReaderRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}

/// The core implementation of the Sandbox Reader functionality
struct SandboxReader;

impl GuestJsonToJson for SandboxReader {
    /// Process the input request and return a response
    ///
    /// # Arguments
    ///
    /// * `input` - A JSON string containing the request
    ///
    /// # Returns
    ///
    /// * `Result<String, PluginError>` - Either a JSON response string or an error
    fn work(&self, input: String) -> Result<String, PluginError> {
        let req = serde_json::from_str::<ReaderRequest>(&input)
            .map_err(|e| PluginError::Json(format!("Invalid input: {} -- {}", e, input)))?;
        match req.action {
            Action::FileRead { file, as_string } => {
                if std::path::Path::new(&file).exists() {
                    // Read the file as bytes
                    let file_content = match std::fs::read(&file) {
                        Ok(content) => content,
                        Err(e) => {
                            return Err(PluginError::Unexpected(format!(
                                "Failed to read file: {}",
                                e
                            )))
                        }
                    };

                    if as_string {
                        // Try to convert the file content to a string
                        let result = match String::from_utf8_lossy(&file_content) {
                            std::borrow::Cow::Borrowed(s) => s.to_string(),
                            std::borrow::Cow::Owned(s) => s,
                        };

                        // Return the result as a string
                        let response = Resp { result };
                        return match serde_json::to_string(&response) {
                            Ok(json) => Ok(json),
                            Err(e) => Err(PluginError::Json(format!(
                                "Failed to serialize response: {}",
                                e
                            ))),
                        };
                    }

                    // Base64 encode the file content
                    let encoded = BASE64_STANDARD.encode(file_content);

                    // Return the result
                    let response = Resp { result: encoded };
                    match serde_json::to_string(&response) {
                        Ok(json) => Ok(json),
                        Err(e) => Err(PluginError::Json(format!(
                            "Failed to serialize response: {}",
                            e
                        ))),
                    }
                } else {
                    Err(PluginError::Unexpected(format!("File not found: {}", file)))
                }
            }
            Action::FileWrite {
                file,
                content,
                write_raw,
                append,
            } => {
                // Decode the base64 content
                let bytes = if write_raw {
                    content.into_bytes()
                } else {
                    match BASE64_STANDARD.decode(&content) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            return Err(PluginError::Unexpected(format!(
                                "Failed to decode base64 content: {}",
                                e
                            )))
                        }
                    }
                };

                // Ensure parent directory exists
                if let Some(parent) = std::path::Path::new(&file).parent() {
                    if !parent.exists() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            return Err(PluginError::Unexpected(format!(
                                "Failed to create parent directory: {}",
                                e
                            )));
                        }
                    }
                }

                // Write or append the bytes to the file based on the append flag
                if append {
                    // Open file in append mode
                    use std::fs::OpenOptions;
                    use std::io::Write;

                    let mut file_handle =
                        match OpenOptions::new().create(true).append(true).open(&file) {
                            Ok(file) => file,
                            Err(e) => {
                                return Err(PluginError::Unexpected(format!(
                                    "Failed to open file for appending: {}",
                                    e
                                )))
                            }
                        };

                    // Append the bytes
                    if let Err(e) = file_handle.write_all(&bytes) {
                        return Err(PluginError::Unexpected(format!(
                            "Failed to append to file: {}",
                            e
                        )));
                    }
                } else {
                    // Write the bytes to the file (overwrite)
                    if let Err(e) = std::fs::write(&file, bytes) {
                        return Err(PluginError::Unexpected(format!(
                            "Failed to write to file: {}",
                            e
                        )));
                    }
                }

                // Return success response
                let response = Resp {
                    result: if append {
                        format!("Successfully appended to file: {}", file)
                    } else {
                        format!("Successfully wrote to file: {}", file)
                    },
                };
                match serde_json::to_string(&response) {
                    Ok(json) => Ok(json),
                    Err(e) => Err(PluginError::Json(format!(
                        "Failed to serialize response: {}",
                        e
                    ))),
                }
            }
            Action::FileDelete { file } => {
                // Check if file exists
                if !std::path::Path::new(&file).exists() {
                    return Err(PluginError::Unexpected(format!("File not found: {}", file)));
                }

                // Delete the file
                if let Err(e) = std::fs::remove_file(&file) {
                    return Err(PluginError::Unexpected(format!(
                        "Failed to delete file: {}",
                        e
                    )));
                }

                // Return success response
                let response = Resp {
                    result: format!("Successfully deleted file: {}", file),
                };
                match serde_json::to_string(&response) {
                    Ok(json) => Ok(json),
                    Err(e) => Err(PluginError::Json(format!(
                        "Failed to serialize response: {}",
                        e
                    ))),
                }
            }
            Action::DirFiles { dir } => {
                // Check if directory exists
                if !std::path::Path::new(&dir).exists() {
                    return Err(PluginError::Unexpected(format!(
                        "Directory not found: {}",
                        dir
                    )));
                }

                // Try to read the directory
                let entries = match std::fs::read_dir(&dir) {
                    Ok(entries) => entries,
                    Err(e) => {
                        return Err(PluginError::Unexpected(format!(
                            "Failed to read directory: {}",
                            e
                        )))
                    }
                };

                // Collect all file paths
                let mut file_paths = Vec::new();
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            if let Some(path) = entry.path().to_str() {
                                file_paths.push(path.to_string());
                            }
                        }
                        Err(e) => {
                            return Err(PluginError::Unexpected(format!(
                                "Failed to read entry: {}",
                                e
                            )))
                        }
                    }
                }

                // Serialize to JSON
                let paths = serde_json::to_value(file_paths).unwrap();
                let response = Resp {
                    result: paths.to_string(),
                };
                match serde_json::to_string(&response) {
                    Ok(json) => Ok(json),
                    Err(e) => Err(PluginError::Json(format!(
                        "Failed to serialize response: {}",
                        e
                    ))),
                }
            }
        }
    }

    /// Creates a new instance of the SandboxReader
    fn new() -> Self {
        Self {}
    }
}

// Export the plugin implementation
export!(SandboxReaderPlugin);
