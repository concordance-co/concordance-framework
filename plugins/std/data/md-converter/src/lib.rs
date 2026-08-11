// Construct the injector plugin interface
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
use crate::plugin::injector::{host::convert, markdown_converter::FileType};
use exports::plugin::injector::guest::{Guest, GuestJsonToJson, Metadata, PluginError, PluginKind};

use base64::prelude::*;

use shared::inlined_schema_for;

use plugin::injector::host::update_status;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ConvertToMd {
    pub file_type: String,
    pub b64_bytes: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Resp {
    pub result: String,
}

pub struct MdConverterPlugin;

impl Guest for MdConverterPlugin {
    type JsonToJson = MarkdownConverter;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Markdown Converter".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "Converts pdf, docx, xlsx, ppt, and html to markdown".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&inlined_schema_for!(ConvertToMd)).unwrap(),
            default_input: serde_json::to_string(&ConvertToMd::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}
pub struct MarkdownConverter;

impl GuestJsonToJson for MarkdownConverter {
    fn work(&self, input: String) -> Result<String, PluginError> {
        let Ok(md) = serde_json::from_str::<ConvertToMd>(&input) else {
            return Err(PluginError::Json("Invalid input".to_string()));
        };

        update_status("Processing file into markdown");

        let raw_bytes = BASE64_STANDARD
            .decode(md.b64_bytes)
            .map_err(|e| PluginError::MdError(format!("Expected base64 encoded bytes: {}", e)))?;

        let file_type = match md.file_type.as_str() {
            "pdf" => FileType::Pdf(raw_bytes),
            "ppt" => FileType::Ppt(raw_bytes),
            "docx" => FileType::Docx(raw_bytes),
            "xlsx" => FileType::Xlsx(raw_bytes),
            "html" => FileType::Html(raw_bytes),
            _ => return Err(PluginError::MdError("Invalid file type".to_string())),
        };

        let res = convert(&file_type)?;
        let resp = serde_json::to_string(&Resp { result: res })
            .map_err(|e| PluginError::Json(format!("Invalid json: {}", e)))?;

        Ok(resp.to_string())
    }

    fn new() -> Self {
        Self {}
    }
}

export!(MdConverterPlugin);
