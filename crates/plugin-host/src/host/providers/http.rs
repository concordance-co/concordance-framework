use crate::injector::http;
use crate::injector::{
    error,
    error::{HttpError, PluginError},
};
use std::time::Duration;

/// HTTP client implementation using reqwest.
///
/// Provides methods for making HTTP requests and handling responses.
#[derive(Debug, Clone)]
pub struct ReqwestHttp {
    /// Timeout duration for HTTP requests
    pub timeout: Duration,
}

impl ReqwestHttp {
    /// Creates a new `ReqwestHttp` instance with a default timeout of 30 seconds.
    ///
    /// # Returns
    ///
    /// A new `ReqwestHttp` instance.
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }

    /// Performs an HTTP GET request.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request to send
    ///
    /// # Returns
    ///
    /// A Result containing either the HTTP response or an error.
    pub async fn get(
        &mut self,
        req: http::HttpRequest,
    ) -> Result<http::HttpResponse, error::PluginError> {
        let client = match reqwest::Client::builder().timeout(self.timeout).build() {
            Ok(client) => client,
            Err(_) => return Err(PluginError::Http(HttpError::InvalidRequest)),
        };

        let mut builder = client.get(req.url);

        for (k, v) in req.headers {
            builder = builder.header(k, v);
        }

        let resp = match builder.send().await {
            Ok(r) => r,
            Err(e) => return Err(PluginError::from(e)),
        };

        let status = resp.status().as_u16();
        let body = match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => return Err(PluginError::from(e)),
        };

        Ok(http::HttpResponse { status, body })
    }

    /// Performs an HTTP POST request.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request to send
    ///
    /// # Returns
    ///
    /// A Result containing either the HTTP response or an error.
    pub async fn post(
        &mut self,
        req: http::HttpRequest,
    ) -> Result<http::HttpResponse, error::PluginError> {
        let client = match reqwest::Client::builder().timeout(self.timeout).build() {
            Ok(client) => client,
            Err(_) => return Err(PluginError::Http(HttpError::InvalidRequest)),
        };

        let mut builder = client.post(req.url);

        for (k, v) in req.headers {
            builder = builder.header(k, v);
        }

        // Add the body to the request
        builder = builder.body(req.body);

        let resp = match builder.send().await {
            Ok(r) => r,
            Err(e) => return Err(PluginError::from(e)),
        };

        let status = resp.status().as_u16();
        let body = match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => return Err(PluginError::from(e)),
        };

        Ok(http::HttpResponse { status, body })
    }
}

impl Default for ReqwestHttp {
    /// Creates a default `ReqwestHttp` instance.
    ///
    /// Equivalent to calling `ReqwestHttp::new()`.
    fn default() -> Self {
        Self::new()
    }
}
