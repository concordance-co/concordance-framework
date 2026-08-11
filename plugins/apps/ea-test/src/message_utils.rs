use crate::plugin::injector::host::log;
use crate::plugin::injector::logger::Level;
use crate::plugin::injector::open_a_i_like::{ContentType, Message, MessageContent};
use tiktoken_rs::o200k_base;

// Function to trim messages to stay under token limit
pub fn trim_messages(messages: &mut Vec<Message>) -> Result<(), String> {
    // Initialize the tokenizer
    let bpe = o200k_base().map_err(|e| format!("Failed to initialize tokenizer: {}", e))?;

    // Define token limit (128k tokens)
    const TOKEN_LIMIT: usize = 128 * 1000;

    // Count total tokens
    let mut total_tokens: usize = messages.iter().map(|m| count_message_tokens(m, &bpe)).sum();

    // Keep system messages regardless of position
    let mut system_messages: Vec<Message> = Vec::new();

    // Trim messages if needed, keeping system messages and recent messages
    if total_tokens > TOKEN_LIMIT {
        log(
            Level::Warn,
            &format!(
                "Message history exceeds token limit ({} > {}). Trimming older messages.",
                total_tokens, TOKEN_LIMIT
            ),
        );

        // Separate system messages from regular messages
        let mut regular_messages: Vec<Message> = Vec::new();

        for message in messages.drain(..) {
            if message.role == "system" || message.role == "developer" {
                system_messages.push(message);
            } else {
                regular_messages.push(message);
            }
        }

        // Calculate tokens used by system messages
        let system_tokens: usize = system_messages
            .iter()
            .map(|m| count_message_tokens(m, &bpe))
            .sum();

        // If system messages alone exceed the limit, we have a problem
        if system_tokens > TOKEN_LIMIT {
            log(
                Level::Error,
                &format!(
                    "System messages alone exceed token limit ({} > {})",
                    system_tokens, TOKEN_LIMIT
                ),
            );
            return Err("System messages exceed token limit".to_string());
        }

        // Now add back messages from newest to oldest until we approach the limit
        let available_tokens = TOKEN_LIMIT - system_tokens;
        let mut used_tokens = 0;

        // Start with the newest messages (end of the vector)
        regular_messages.reverse();

        let mut kept_messages = Vec::new();

        for message in regular_messages {
            let message_tokens = count_message_tokens(&message, &bpe);

            if used_tokens + message_tokens <= available_tokens {
                kept_messages.push(message);
                used_tokens += message_tokens;
            } else {
                // We can't fit this message, stop adding
                break;
            }
        }

        // Reverse again to maintain chronological order
        kept_messages.reverse();

        // Combine system messages and kept regular messages
        messages.extend(system_messages);
        messages.extend(kept_messages);

        total_tokens = system_tokens + used_tokens;
        log(
            Level::Info,
            &format!(
                "After trimming: {} tokens, {} messages",
                total_tokens,
                messages.len()
            ),
        );
    }

    Ok(())
}

// Helper function to count tokens in a message
fn count_message_tokens(message: &Message, bpe: &tiktoken_rs::CoreBPE) -> usize {
    let role_tokens = bpe.encode_with_special_tokens(&message.role).len();

    let content_tokens = match &message.content {
        ContentType::Single(MessageContent::Content(text)) => {
            bpe.encode_with_special_tokens(text).len()
        }
        _ => 0, // Handle other content types if needed
    };

    // Add a small buffer for the message structure itself
    role_tokens + content_tokens + 4
}
