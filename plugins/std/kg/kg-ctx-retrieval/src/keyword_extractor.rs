use crate::models::*;
use crate::plugin::injector::error::PluginError;
use crate::plugin::injector::{
    host::{log, new_client},
    logger::Level,
    open_a_i_like::{ChatConfig, ContentType, Message, MessageContent},
};
use shared::{
    response_format_inlined_schema_for, types::LLMConfig, utils::llm_json_response_cleanup,
};

/// Extracts high-level conceptual and low-level entity keywords from user queries.
///
/// This struct provides functionality to analyze input queries and identify
/// both abstract conceptual keywords and specific entity keywords that can be
/// used for improved knowledge retrieval and context understanding.
pub struct KeywordExtractor {}

impl KeywordExtractor {
    /// Creates a new instance of the KeywordExtractor.
    ///
    /// # Returns
    /// A new KeywordExtractor instance.
    pub fn new() -> Self {
        Self {}
    }

    /// Extracts high-level conceptual keywords and low-level entity keywords from a user query.
    ///
    /// This method uses an LLM to analyze the input query and identify two types of keywords:
    /// 1. High-level keywords: Abstract concepts, categories, or domains related to the query
    /// 2. Low-level keywords: Specific entities, names, places, or terms mentioned in the query
    ///
    /// # Arguments
    /// * `request` - The context request containing the user query and LLM configuration
    ///
    /// # Returns
    /// * `Ok((Vec<String>, Vec<String>))` - A tuple containing vectors of high-level and low-level keywords
    /// * `Err(PluginError)` - An error if keyword extraction fails
    ///
    /// # Examples
    /// For a query like "Tell me about the history of the Eiffel Tower":
    /// - High-level keywords might include: ["history", "architecture", "landmarks"]
    /// - Low-level keywords might include: ["Eiffel Tower", "Paris", "France"]
    pub fn extract_keywords(
        &self,
        request: &ContextRequest,
        llm_config: &LLMConfig,
    ) -> Result<(Vec<String>, Vec<String>), PluginError> {
        log(
            Level::Info,
            &format!("Extracting keywords for query: {}", request.query),
        );

        // Define the keyword extraction prompts
        let examples = r#"
        Example 1:
        Query: "What is the weather in New York today?"
        {
          "highLevelKeywords": ["weather", "meteorology", "forecast"],
          "lowLevelKeywords": ["New York", "today"]
        }

        Example 2:
        Query: "Tell me about the history of the Eiffel Tower"
        {
          "highLevelKeywords": ["history", "architecture", "landmarks"],
          "lowLevelKeywords": ["Eiffel Tower", "Paris", "France"]
        }
        "#;

        let language = "English";

        // Build the prompt for keyword extraction
        let kw_prompt = format!(
            r#"Extract high-level conceptual keywords and low-level entity keywords from the following query for knowledge retrieval.

            High-level keywords are abstract concepts, categories, or domains related to the query (e.g., "history", "physics", "medicine").
            Low-level keywords are specific entities, names, places, or terms mentioned in the query (e.g., "Eiffel Tower", "Newton", "cancer").

            Respond with a JSON object containing two arrays:
            1. "high_level_keywords": Conceptual or categorical terms relevant to the query
            2. "low_level_keywords": Specific entities or precise terms from the query

            Query: "{query}"

            {examples}

            Language: {language}

            JSON response:"#,
            query = request.query,
            examples = examples,
            language = language,
        );

        log(
            Level::Debug,
            &format!("Keyword extraction prompt: {}", kw_prompt),
        );

        // Set up LLM configuration
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
                    "name": "keywords_extraction_response",
                    "strict": true,
                    "required": ["keywords_extraction_response"],
                    "schema": response_format_inlined_schema_for!(KeywordsExtractionResponse)
                }))
                .unwrap(),
            ),
        };

        chat_config.messages.push(Message {
            role: "user".to_string(),
            content: ContentType::Single(MessageContent::Content(kw_prompt)),
            tool_calls: None,
            tool_call_id: None,
        });

        // Initialize a client for LLM calls
        let client = new_client(&llm_config.base_url, &llm_config.api_key)?;

        // Call the LLM service
        let llm_result = match client.chat_create(&chat_config) {
            Ok(response) => {
                if response.choices.is_empty() {
                    return Err(PluginError::ChatCompletion(
                        "No response from LLM".to_string(),
                    ));
                }
                response.choices[0]
                    .message
                    .content
                    .clone()
                    .unwrap_or_default()
            }
            Err(e) => return Err(e),
        };

        // Clean up the response to extract JSON content
        log(Level::Info, &format!("Raw LLM response: {}", llm_result));
        let clean_resp = llm_json_response_cleanup(&llm_result);

        // Parse the LLM response
        match serde_json::from_str::<KeywordsExtractionResponse>(&clean_resp) {
            Ok(keywords_data) => {
                log(
                    Level::Info,
                    &format!(
                        "Extracted keywords - High-level: {:?}, Low-level: {:?}",
                        keywords_data.high_level_keywords, keywords_data.low_level_keywords
                    ),
                );

                Ok((
                    keywords_data.high_level_keywords,
                    keywords_data.low_level_keywords,
                ))
            }
            Err(e) => {
                log(
                    Level::Error,
                    &format!(
                        "Failed to parse LLM response: {}, raw: {}, cleaned: {}",
                        e, llm_result, clean_resp
                    ),
                );
                Err(PluginError::Json(format!(
                    "Failed to parse keywords response: {} -- {}",
                    e, llm_result
                )))
            }
        }
    }
}

impl Default for KeywordExtractor {
    fn default() -> Self {
        Self::new()
    }
}
