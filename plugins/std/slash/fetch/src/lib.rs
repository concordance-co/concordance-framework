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

// host capabilities
use crate::plugin::injector::{
    host::{log, update_status},
    logger::Level,
};

use html_to_markdown::{convert_html_to_markdown, markdown, TagHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::{
    inlined_schema_for,
    types::{SlashCommandInput, SlashCommandOutput},
    with_examples_inlined_schema_for,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// /// The request structure for the Fetch plugin
// #[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
// pub struct FetchRequest {
//     /// The URL to fetch
//     pub url: String,
//     /// Optional HTTP method (defaults to GET if not specified)
//     pub method: Option<String>,
//     /// Optional HTTP headers to include in the request
//     pub headers: Option<HashMap<String, String>>,
//     /// Optional request body for POST, PUT requests
//     pub body: Option<String>,
//     /// Whether to return the raw response (true) or try to extract readable text (false)
//     pub raw_response: Option<bool>,
// }

/// The content type of the HTTP response
enum ContentType {
    Html,
    Plaintext,
    Json,
}

struct FetchPlugin;

impl Guest for FetchPlugin {
    type JsonToJson = WebFetcher;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Fetch".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "Fetches the content of a web page using HTTP requests".to_string(),
            kind: PluginKind::SlashCommand,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&inlined_schema_for!(SlashCommandInput)).unwrap(),
            default_input: serde_json::to_string(&SlashCommandInput::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(SlashCommandOutput)).unwrap(),
        }
    }
}

struct WebFetcher;

impl GuestJsonToJson for WebFetcher {
    fn work(&self, request: String) -> Result<String, PluginError> {
        self.fetch_with_waki(request)
    }

    fn new() -> Self {
        Self {}
    }
}

impl WebFetcher {
    /// Fetch a URL using waki
    fn fetch_with_waki(&self, url: String) -> Result<String, PluginError> {
        update_status(&format!("Fetching webpage: {}", &url));
        log(Level::Info, &format!("Fetching URL: {}", url));

        // Determine the HTTP method to use
        let method = waki::Method::Get;

        // Build the request
        let mut request_builder = waki::RequestBuilder::new(method.clone(), &url);

        request_builder = request_builder.header("User-Agent", "Concordance/1.0");

        // log(Level::Info, &format!("Request: {:?}", request_builder));
        // Send the request
        let response = request_builder
            .send()
            .map_err(|e| PluginError::Unexpected(format!("HTTP request failed: {}", e)))?;

        // Get status code
        let status_code = response.status_code();
        // Extract Content-Type header
        let headers = response.headers().clone();
        let Some(content_type) = headers.get("content-type") else {
            return Err(PluginError::Unexpected(
                "Missing Content-Type header".to_string(),
            ));
        };
        // Get the response body
        let body = response
            .body()
            .map_err(|e| PluginError::Unexpected(format!("Failed to read response body: {}", e)))?;

        if !(200..300).contains(&status_code) {
            return Err(PluginError::Unexpected(format!(
                "HTTP request failed with status code: {}. Body: {}",
                status_code,
                String::from_utf8_lossy(&body)
            )));
        }

        // Process based on Content-Type
        let Ok(content_type) = content_type.to_str() else {
            return Err(PluginError::Unexpected(
                "Invalid Content-Type header".to_string(),
            ));
        };
        let content_type = match content_type {
            "text/html" => ContentType::Html,
            "text/plain" => ContentType::Plaintext,
            "application/json" => ContentType::Json,
            _ => ContentType::Html,
        };

        let res = match content_type {
            ContentType::Html => {
                let mut handlers: Vec<TagHandler> = vec![
                    Rc::new(RefCell::new(markdown::WebpageChromeRemover)),
                    Rc::new(RefCell::new(markdown::ParagraphHandler)),
                    Rc::new(RefCell::new(markdown::HeadingHandler)),
                    Rc::new(RefCell::new(markdown::ListHandler)),
                    Rc::new(RefCell::new(markdown::TableHandler::new())),
                    Rc::new(RefCell::new(markdown::StyledTextHandler)),
                ];
                if url.contains("wikipedia.org") {
                    use html_to_markdown::structure::wikipedia;

                    handlers.push(Rc::new(RefCell::new(wikipedia::WikipediaChromeRemover)));
                    handlers.push(Rc::new(RefCell::new(wikipedia::WikipediaInfoboxHandler)));
                    handlers.push(Rc::new(
                        RefCell::new(wikipedia::WikipediaCodeHandler::new()),
                    ));
                } else {
                    handlers.push(Rc::new(RefCell::new(markdown::CodeHandler)));
                }

                convert_html_to_markdown(&body[..], &mut handlers).map_err(|e| {
                    PluginError::Unexpected(format!("Unable to convert HTML to Markdown: {}", e))
                })
            }
            ContentType::Plaintext => {
                let v: String = String::from_utf8_lossy(&body).to_string();
                Ok(v)
            }
            ContentType::Json => {
                let json: serde_json::Value = serde_json::from_slice(&body)
                    .map_err(|e| PluginError::Json(format!("Failed to deserialize JSON: {}", e)))?;

                Ok(format!(
                    "```json\n{}\n```",
                    serde_json::to_string_pretty(&json).map_err(|e| PluginError::Json(format!(
                        "Failed to serialize JSON: {}",
                        e
                    )))?
                ))
            }
        }?;
        Ok(serde_json::to_string(&SlashCommandOutput { output: res }).unwrap())
    }
}

export!(FetchPlugin);
