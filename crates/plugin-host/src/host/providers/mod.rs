//! Provider implementations for various services used by plugins.
//!
//! This module contains implementations for different service providers that
//! plugins can utilize:
//! - `convert`: File format conversion and document processing
//! - `http`: HTTP client functionality for making web requests
//! - `llm`: Large Language Model client implementations
//! - `vectordb`: Vector database connectivity and operations

/// Conversion providers for transforming documents between formats
pub mod convert;
/// HTTP client providers for making web requests
pub mod http;
/// LLM providers for interacting with language models
pub mod llm;
/// Vector database providers for similarity search and embeddings storage
pub mod vectordb;

pub use convert::MdConverter;
pub use http::ReqwestHttp;
pub use llm::{ChatSession, Client, OpenAIConfig};
pub use vectordb::DbConn;

// Reexport any error conversions or shared utilities here
use crate::injector::error::{HttpError, PluginError};

impl From<reqwest::Error> for PluginError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_connect() || value.is_status() {
            return PluginError::Http(HttpError::Network);
        } else if value.is_timeout() {
            return PluginError::Http(HttpError::Timeout);
        }
        PluginError::Http(HttpError::InvalidRequest)
    }
}
