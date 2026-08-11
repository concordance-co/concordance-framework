use tiktoken_rs::o200k_base;

/// Truncates a vector of string chunks to fit within a specified token limit.
///
/// This function uses the OpenAI o200k tokenizer model to ensure the total
/// number of tokens across all chunks does not exceed the specified maximum.
///
/// # Arguments
///
/// * `chunks` - A vector of string chunks to be potentially truncated.
/// * `max_tokens` - The maximum number of tokens allowed in the result.
///
/// # Returns
///
/// * `Ok(Vec<String>)` - The truncated vector of string chunks.
/// * `Err(String)` - An error message if tokenization fails.
///
/// # Behavior
///
/// The function processes chunks sequentially, adding them to the result
/// until the token limit is reached. If a chunk would exceed the limit,
/// it includes as many tokens from that chunk as possible without exceeding
/// the limit, then stops processing.
pub fn truncate_by_token_size(
    chunks: Vec<String>,
    max_tokens: usize,
) -> Result<Vec<String>, String> {
    let bpe = o200k_base().map_err(|e| format!("Failed to initialize tokenizer: {}", e))?;
    let mut result = Vec::new();

    let mut total_tokens = 0;
    for chunk in chunks {
        let tokens = bpe.encode_with_special_tokens(&chunk);
        if total_tokens + tokens.len() > max_tokens {
            // Add as many tokens as will fit
            if max_tokens > total_tokens {
                let partial = bpe
                    .split_by_token_iter(&chunk, true)
                    .take(max_tokens - total_tokens)
                    .collect::<Result<Vec<String>, _>>()
                    .map_err(|e| format!("Failed to split tokens: {}", e))?
                    .join("");

                result.push(partial);
            }
            break;
        }
        total_tokens += tokens.len();
        result.push(chunk);
    }

    Ok(result)
}
