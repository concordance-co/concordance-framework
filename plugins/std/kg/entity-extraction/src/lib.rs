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

use crate::plugin::injector::{
    error::FsError,
    host::{log, new_client, update_status},
    logger::Level,
    open_a_i_like::{ChatConfig, Client, ContentType, Message, MessageContent},
};

use crate::exports::plugin::injector::guest::{
    Guest, GuestJsonToJson, Metadata, PluginError, PluginKind,
};

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::{
    inlined_schema_for, response_format_inlined_schema_for, types::LLMConfig,
    utils::llm_json_response_cleanup, with_examples_inlined_schema_for, TryFromEnvVar,
};
use std::sync::LazyLock;

// Include the prompt files
pub const EXTRACTION_BASE_PROMPT: &str = include_str!("../prompts/base.txt");

pub const EXAMPLE_0: &str = include_str!("../prompts/example0.txt");
pub const EXAMPLE_1: &str = include_str!("../prompts/example1.txt");
pub const EXAMPLE_2: &str = include_str!("../prompts/example2.txt");

pub static EXAMPLES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| vec![EXAMPLE_0, EXAMPLE_1, EXAMPLE_2]);
/// A filter for preprocessing input text before entity extraction.
/// Uses regex patterns to find and replace text content.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Filter {
    /// The regex pattern to search for in the input text
    pub regex_find: String,
    /// The replacement pattern to use when a match is found
    pub regex_replace: String,
}

/// Configuration for customizing prompts used in entity extraction.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct PromptConfig {
    /// Optional system prompt to set context for the LLM
    pub system_prompt: Option<String>,
    /// Optional custom base prompt to replace the default one
    pub base_prompt: Option<String>,
    /// Optional list of examples to use instead of the default examples
    pub examples: Option<Vec<String>>,
    /// Optional prompt for continuing extraction in multi-part processing
    pub continuation_prompt: Option<String>,
}

/// Configuration for the entity extraction process.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ExtractionConfig {
    /// Regex find/replace strings to be applied to input strings before entity extraction
    pub input_filters: Option<Vec<Filter>>,
    /// Optional list of specific entity types to extract
    pub entity_types: Option<Vec<String>>,
}

/// Represents an input text to process for entity extraction.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Input {
    /// Optional identifier for the input, used for caching and source tracking
    pub id: Option<String>,
    /// The text content to extract entities from
    pub content: String,
}

/// The main request structure for the entity extraction process.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct EntityExtractorRequest {
    /// List of input texts to process for entity extraction
    pub strs: Vec<Input>,
    /// Whether to use caching for previously processed inputs (defaults to true if not specified)
    pub use_cache: Option<bool>,
    /// Optional LLM configuration overrides - in general should be left blank
    pub llm_config: Option<LLMConfig>,
    /// Optional configuration for the extraction process
    pub extraction_config: Option<ExtractionConfig>,
    /// Optional configuration for customizing prompts
    pub prompt_config: Option<PromptConfig>,
}

/// The response structure containing extraction results.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Resp {
    /// List of extraction results, one for each input
    pub result: Vec<Root>,
}

/// The root structure containing extraction results for a single input.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Root {
    /// The entity extraction results
    pub entity_extraction: EntityExtraction,
}

/// Contains the extracted entities, relationships, and keywords.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EntityExtraction {
    /// List of entities extracted from the text
    pub entities: Vec<Entity>,
    /// List of relationships between the extracted entities
    pub relationships: Vec<Relationship>,
    /// High-level keywords extracted from the content
    pub content_keywords: ContentKeywords,
}

/// Represents a single entity extracted from the text.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Entity {
    /// The name of the entity
    pub name: String,
    /// The type or category of the entity
    #[serde(rename = "type")]
    pub type_field: String,
    /// A description of the entity based on context
    pub description: String,
    /// Reference to the source document where this entity was found
    pub doc_source: Option<String>,
}

/// Represents a relationship between two entities.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Relationship {
    /// The source entity in the relationship
    pub source: String,
    /// The target entity in the relationship
    pub target: String,
    /// A description of how the entities are related
    pub description: String,
    /// Keywords that characterize this relationship
    pub keywords: String,
    /// A numeric value indicating the strength of the relationship
    pub strength: usize,
    /// Reference to the source document where this relationship was found
    pub doc_source: Option<String>,
}

/// Contains high-level keywords extracted from the content.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContentKeywords {
    /// A string of high-level keywords that summarize the content
    pub high_level_keywords: String,
}

pub struct EntityExtractorPlugin;

impl Guest for EntityExtractorPlugin {
    type JsonToJson = EntityExtractor;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Entity Extraction".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description:
                "Given a list of strings and LLM configuration, extracts entities using an LLM."
                    .to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![("llm_config".to_string(), "LLM_CONFIG".to_string())],
            input_schema: serde_json::to_string(&with_examples_inlined_schema_for!(
                EntityExtractorRequest,
                EntityExtractorRequest::default()
            ))
            .unwrap(),
            default_input: serde_json::to_string(&EntityExtractorRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}

fn build_examples_text(prompt_config: Option<&PromptConfig>) -> String {
    let mut examples_text = String::new();

    let examples = match prompt_config {
        Some(config) if config.examples.is_some() => config.examples.as_ref().unwrap(),
        _ => {
            examples_text
                .push_str("\n######################\n---Examples---\n######################\n");
            return EXAMPLES
                .iter()
                .enumerate()
                .fold(examples_text, |mut acc, (i, example)| {
                    use std::fmt::Write;
                    let _ = write!(acc, "Example {}:\n{}\n", i + 1, example);
                    acc
                });
        }
    };

    if examples.is_empty() {
        return String::new();
    }

    examples_text.push_str("\n######################\n---Examples---\n######################\n");
    for (i, example) in examples.iter().enumerate() {
        examples_text.push_str(&format!("Example {}:\n{}\n", i + 1, example));
    }

    examples_text
}

fn load_or_create_cache() -> Result<serde_json::Map<String, serde_json::Value>, PluginError> {
    // Create cache directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all("./caches") {
        return Err(PluginError::Fs(FsError::Other(format!(
            "Failed to create cache directory: {}",
            e
        ))));
    }

    // Try to open existing cache file or create a new one
    let cache_path = "./caches/entity-cache.json";
    let cache_data = match std::fs::File::open(cache_path) {
        Ok(file) => serde_json::from_reader(&file).unwrap_or_else(|_| serde_json::Map::new()),
        Err(_) => {
            // File doesn't exist, create it
            if let Err(e) = std::fs::File::create(cache_path) {
                log(Level::Error, &format!("Failed to create cache file: {}", e));
            }
            serde_json::Map::new()
        }
    };

    Ok(cache_data)
}

fn save_cache(cache_data: &serde_json::Map<String, serde_json::Value>) -> Result<(), PluginError> {
    let cache_path = "./caches/entity-cache.json";
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(cache_path)
        .map_err(|e| {
            PluginError::Fs(FsError::Other(format!(
                "Failed to open cache file for writing: {}",
                e
            )))
        })?;

    serde_json::to_writer(file, cache_data).map_err(|e| {
        PluginError::Fs(FsError::Other(format!(
            "Failed to write to cache file: {}",
            e
        )))
    })
}

fn apply_regex_filters(input: &str, filters: &[Filter]) -> String {
    let mut processed = input.to_string();

    for filter in filters {
        match Regex::new(&filter.regex_find) {
            Ok(regex) => {
                processed = regex
                    .replace_all(&processed, &filter.regex_replace)
                    .to_string();
            }
            Err(e) => {
                log(
                    Level::Error,
                    &format!("Invalid regex pattern '{}': {}", filter.regex_find, e),
                );
            }
        }
    }

    processed
}

fn extract_entity_for_input(
    client: &Client,
    input_str: &Input,
    llm_config: &LLMConfig,
    extraction_config: &Option<ExtractionConfig>,
    prompt_config: &Option<PromptConfig>,
    examples_text: &str,
) -> Result<Root, PluginError> {
    // Apply regex filters to the input string
    let processed_str = match extraction_config {
        Some(config) if config.input_filters.is_some() => {
            apply_regex_filters(&input_str.content, config.input_filters.as_ref().unwrap())
        }
        _ => input_str.content.clone(),
    };

    // Create a chat config for this input
    let mut chat_config = ChatConfig {
        model: llm_config.model_name.clone(),
        temperature: llm_config.temperature,
        max_tokens: llm_config.max_tokens,
        top_p: llm_config.top_p,
        top_k: llm_config.top_k,
        tools: llm_config.tools.clone(),
        tool_choice: None,
        messages: vec![],
        streaming: Some(false),
        response_schema: Some(
            serde_json::to_string(&serde_json::json!({
                "name": "root",
                "strict": true,
                "schema": response_format_inlined_schema_for!(Root)
            }))
            .unwrap(),
        ),
    };

    // Add system prompt if available
    if let Some(config) = prompt_config {
        if let Some(system_prompt) = &config.system_prompt {
            chat_config.messages.push(Message {
                role: "system".to_string(),
                content: ContentType::Single(MessageContent::Content(system_prompt.clone())),
                tool_calls: None,
                tool_call_id: None,
            });
        }
    }

    // Construct the user prompt with base prompt, examples, and the processed text
    let base_prompt = prompt_config
        .as_ref()
        .and_then(|config| config.base_prompt.as_ref())
        .cloned()
        .unwrap_or_else(|| EXTRACTION_BASE_PROMPT.to_string());

    let prompt = format!(
        "{}\n{}\n\nInput Text:\n{}",
        base_prompt, examples_text, processed_str
    );

    // Add the prompt as user message
    chat_config.messages.push(Message {
        role: "user".to_string(),
        content: ContentType::Single(MessageContent::Content(prompt)),
        tool_calls: None,
        tool_call_id: None,
    });

    // Call the LLM with this chat configuration
    let mut response = client.chat_create(&chat_config)?;

    let choice = response.choices.swap_remove(0);
    let Some(mut text_res) = choice.message.content else {
        return Err(PluginError::ChatCompletion(
            "No message content from LLM".to_string(),
        ));
    };

    // Clean up the response to ensure it's valid JSON
    text_res = llm_json_response_cleanup(&text_res);

    // Parse the response into our Root struct
    let mut res: Root = serde_json::from_str(&text_res).map_err(|e| {
        PluginError::Json(format!(
            "The LLM failed to follow the entity json format: {e}"
        ))
    })?;

    // Add source to each entity and relationship if ID is available
    if let Some(src) = &input_str.id {
        for entity in &mut res.entity_extraction.entities {
            entity.doc_source = Some(src.clone());
        }

        for rel in &mut res.entity_extraction.relationships {
            rel.doc_source = Some(src.clone());
        }
    }

    Ok(res)
}

pub struct EntityExtractor;
impl GuestJsonToJson for EntityExtractor {
    fn work(&self, input: String) -> Result<String, PluginError> {
        let req: EntityExtractorRequest = serde_json::from_str(&input)
            .map_err(|e| PluginError::Json(format!("Invalid input json: {}", e)))?;

        update_status("Performing entity extraction");

        let llm_config = match req.llm_config {
            Some(ref config) => config.clone(),
            None => LLMConfig::try_from_env_var("LLM_CONFIG")
                .map_err(|e| PluginError::EnvVar(format!("Failed to load LLM_CONFIG: {}", e)))?,
        };

        // Create a LLM client
        let client = new_client(&llm_config.base_url, &llm_config.api_key)?;

        // Build examples text once for all inputs
        let examples_text = build_examples_text(req.prompt_config.as_ref());

        let use_cache = req.use_cache.unwrap_or(true);
        let mut resp = Resp { result: vec![] };

        // Load or create cache if we're using it
        let mut cache_data = if use_cache {
            load_or_create_cache()?
        } else {
            serde_json::Map::new()
        };

        // Process each input string
        let total_inputs = req.strs.len();
        for (i, input_str) in req.strs.iter().enumerate() {
            update_status(&format!(
                "Performing entity extraction for chunk: {} / {}",
                i + 1,
                total_inputs
            ));

            // Check cache first if enabled
            let cache_key = input_str
                .id
                .clone()
                .unwrap_or_else(|| input_str.content.clone());

            if use_cache {
                if let Some(cached_result) = cache_data.get(&cache_key) {
                    if let Ok(cached_root) = serde_json::from_value(cached_result.clone()) {
                        resp.result.push(cached_root);
                        continue;
                    }
                }
            }

            // Process this input and extract entities
            match extract_entity_for_input(
                &client,
                input_str,
                &llm_config,
                &req.extraction_config,
                &req.prompt_config,
                &examples_text,
            ) {
                Ok(res) => {
                    // Update cache if enabled
                    if use_cache {
                        cache_data.insert(cache_key, serde_json::to_value(res.clone()).unwrap());
                    }

                    resp.result.push(res);
                }
                Err(e) => {
                    log(
                        Level::Error,
                        &format!("Error processing input {}: {}", i, e),
                    );
                    // Continue with other inputs rather than failing the whole operation
                }
            }
        }

        // Save the updated cache if we're using it
        if use_cache {
            if let Err(e) = save_cache(&cache_data) {
                log(Level::Error, &format!("Failed to save cache: {}", e));
            }
        }

        // Return the results
        update_status("Finished entity extraction");
        serde_json::to_string(&resp).map_err(|e| PluginError::Json(e.to_string()))
    }

    fn new() -> Self {
        Self {}
    }
}

export!(EntityExtractorPlugin);
