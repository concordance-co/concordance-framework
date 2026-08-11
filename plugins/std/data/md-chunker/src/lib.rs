//! This module provides functionality to chunk markdown documents into smaller pieces
//! based on target sizes. It's designed to optimize text for efficient processing
//! by language models or other text analysis tools.

// Construct the injector plugin interface
#[cfg(target_arch = "wasm32")]
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
#[cfg(target_arch = "wasm32")]
use exports::plugin::injector::guest::{Guest, GuestJsonToJson, Metadata, PluginError, PluginKind};
#[cfg(target_arch = "wasm32")]
use plugin::injector::host::update_status;

use markdown::{mdast::Node, to_mdast, ParseOptions};
#[cfg(target_arch = "wasm32")]
use shared::inlined_schema_for;

use shared::TryFromEnvVar;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tiktoken_rs::o200k_base;

/// Input parameters for the markdown chunking operation
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ChunkMd {
    /// Name of the file being processed
    pub file_name: String,
    /// Markdown content to be chunked
    pub md: String,
    /// Maximum target size for each chunk in tokens
    pub target_chunk_size: Option<usize>,
    /// Minimum target size for each chunk in tokens
    pub target_min_chunk_size: Option<usize>,
}

/// Response structure containing the chunked results
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Resp {
    /// List of chunks created from the original markdown
    pub result: Vec<Chunk>,
}

/// A single chunk of markdown content
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Chunk {
    /// Unique identifier for the chunk
    pub id: String,
    /// The markdown content of this chunk
    pub content: String,
}

/// Plugin implementation for the markdown chunker
pub struct MdChunkerPlugin;

#[cfg(target_arch = "wasm32")]
impl Guest for MdChunkerPlugin {
    type JsonToJson = MdChunker;

    /// Provides metadata about the plugin
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Markdown Chunker".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "Given some markdown, chunk it into smaller pieces based on target_chunk_size and target_min_chunk_size".to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![("target_chunk_size".to_string(), "MD_CHUNK_TARGET_SIZE".to_string()), ("target_min_chunk_size".to_string(), "MD_CHUNK_MIN_SIZE".to_string())],
            input_schema: serde_json::to_string(&inlined_schema_for!(ChunkMd)).unwrap(),
            default_input: serde_json::to_string(&ChunkMd::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}

/// Implementation of the chunking algorithm
pub struct MdChunker;

/// Type alias for a chunk's start and end positions and token count
/// Format: ((start_position, end_position), token_count)
type ChunkInfo = ((usize, usize), usize);

#[cfg(target_arch = "wasm32")]
impl GuestJsonToJson for MdChunker {
    /// Main entry point for the chunking process
    ///
    /// Takes a JSON string input, processes the markdown into chunks, and returns
    /// the result as a JSON string
    fn work(&self, input: String) -> Result<String, PluginError> {
        // Parse the input JSON into our ChunkMd struct
        let md = serde_json::from_str::<ChunkMd>(&input)
            .map_err(|e| PluginError::Json(format!("Invalid input: {}", e)))?;

        update_status("Converting markdown into chunks...");

        // Process the markdown into chunks
        let chunks = chunk_markdown(&md)
            .map_err(|e| PluginError::Json(format!("Error chunking markdown: {}", e)))?;

        // Create the result object with the chunks
        let result = create_result(&md, chunks);

        // Serialize the result back to JSON
        serde_json::to_string(&result)
            .map_err(|e| PluginError::Json(format!("Error serializing result: {}", e)))
    }

    /// Creates a new instance of the MdChunker
    fn new() -> Self {
        Self {}
    }
}

/// Chunks markdown content based on target sizes
///
/// This is the main chunking algorithm that:
/// 1. Parses markdown into an AST
/// 2. Finds section boundaries at headings
/// 3. Processes each section into appropriate chunks
/// 4. Optimizes chunks by merging small ones
///
/// # Arguments
/// * `md` - The ChunkMd struct containing markdown and target sizes
///
/// # Returns
/// * `Result<Vec<ChunkInfo>, String>` - A vector of chunk information or an error
fn chunk_markdown(md: &ChunkMd) -> Result<Vec<ChunkInfo>, String> {
    // Initialize tokenizer for token counting
    let bpe = o200k_base().map_err(|e| format!("Failed to initialize tokenizer: {}", e))?;

    let target_chunk_size = match md.target_chunk_size {
        Some(size) => size,
        None => usize::try_from_env_var("MD_CHUNK_TARGET_SIZE").unwrap_or(1000),
    };
    let target_min_chunk_size = match md.target_min_chunk_size {
        Some(size) => size,
        None => usize::try_from_env_var("MD_CHUNK_MIN_SIZE").unwrap_or(150),
    };

    // Parse markdown to AST using GitHub Flavored Markdown settings
    let gfm = ParseOptions::gfm();
    let ast = to_mdast(&md.md, &gfm).map_err(|e| format!("Failed to parse markdown: {}", e))?;

    let children = ast.children().ok_or("No children in AST")?;

    // First pass: Find section boundaries (at heading nodes)
    let sections = find_sections(children)?;

    // Second pass: Create chunks from sections
    let mut chunks = Vec::new();

    for &(start, end) in &sections {
        let section_content = &md.md[start..end];
        let token_count = bpe.encode_with_special_tokens(section_content).len();

        // If section fits within target size, keep it whole
        if token_count <= target_chunk_size {
            chunks.push(((start, end), token_count));
            continue;
        }

        // Otherwise, break it down further
        let section_nodes: Vec<_> = children
            .iter()
            .filter(|node| {
                node.position()
                    .is_some_and(|pos| pos.start.offset >= start && pos.end.offset <= end)
            })
            .collect();

        process_section_nodes(
            md,
            target_chunk_size,
            target_min_chunk_size,
            &mut chunks,
            section_nodes,
            start,
            end,
            &bpe,
        )?;
    }

    // Optimize chunks - merge small chunks when appropriate
    let optimized_chunks = optimize_chunks(chunks, target_chunk_size, target_min_chunk_size);

    Ok(optimized_chunks)
}

/// Identifies section boundaries in the markdown AST
///
/// Sections are defined by heading elements in the markdown.
/// Each heading starts a new section that continues until the next heading.
///
/// # Arguments
/// * `children` - Nodes in the markdown AST
///
/// # Returns
/// * `Result<Vec<(usize, usize)>, String>` - List of section boundaries as (start, end) offsets
fn find_sections(children: &[Node]) -> Result<Vec<(usize, usize)>, String> {
    let mut sections = Vec::new();
    let mut current_chunk = (0, 0);

    for child in children {
        let position = child
            .position()
            .ok_or_else(|| format!("Node missing position: {:?}", child))?;

        if let Node::Heading(_) = child {
            if current_chunk != (0, 0) {
                sections.push(current_chunk);
            }
            current_chunk = (position.start.offset, position.end.offset);
        } else {
            current_chunk.1 = position.end.offset;
        }
    }

    if current_chunk != (0, 0) {
        sections.push(current_chunk);
    }

    Ok(sections)
}

/// Processes nodes within a section to create appropriate chunks
///
/// This function breaks down larger sections into smaller chunks based on
/// the target chunk sizes and special handling for certain node types.
///
/// # Arguments
/// * `md` - The ChunkMd struct with target sizes
/// * `chunks` - Vector to store the created chunks
/// * `section_nodes` - Nodes within the current section
/// * `section_start` - Start offset of the section
/// * `section_end` - End offset of the section
/// * `bpe` - Tokenizer for calculating token counts
///
/// # Returns
/// * `Result<(), String>` - Success or an error message
#[allow(clippy::too_many_arguments)]
fn process_section_nodes(
    md: &ChunkMd,
    target_chunk_size: usize,
    target_min_chunk_size: usize,
    chunks: &mut Vec<ChunkInfo>,
    section_nodes: Vec<&Node>,
    section_start: usize,
    section_end: usize,
    bpe: &tiktoken_rs::CoreBPE,
) -> Result<(), String> {
    let mut current_chunk_start = section_start;
    let mut current_chunk_size = 0;

    for node in section_nodes {
        let node_pos = node
            .position()
            .ok_or_else(|| format!("Node missing position: {:?}", node))?;
        let node_content = &md.md[node_pos.start.offset..node_pos.end.offset];
        let node_tokens = bpe.encode_with_special_tokens(node_content).len();

        // Special case: Keep tables together
        let is_table = matches!(node, Node::Table(_));

        // Start a new chunk if adding this node would exceed target size
        // and current chunk meets minimum size
        let too_large = current_chunk_size + node_tokens > target_chunk_size;
        let large_enough = current_chunk_size >= target_min_chunk_size;
        if !is_table && too_large && large_enough {
            chunks.push((
                (current_chunk_start, node_pos.start.offset),
                current_chunk_size,
            ));
            current_chunk_start = node_pos.start.offset;
            current_chunk_size = node_tokens;
        } else {
            current_chunk_size += node_tokens;
        }
    }

    // Add the final chunk if it's not empty
    if current_chunk_size > 0 {
        chunks.push(((current_chunk_start, section_end), current_chunk_size));
    }

    Ok(())
}

/// Optimizes chunks by merging small chunks with their neighbors
///
/// This ensures that all chunks meet the minimum size requirement when possible
/// by merging them with adjacent chunks that have space.
///
/// # Arguments
/// * `chunks` - Vector of chunks to optimize
/// * `target_size` - Maximum target size for chunks
/// * `min_size` - Minimum target size for chunks
///
/// # Returns
/// * `Vec<ChunkInfo>` - Optimized chunks with small chunks merged when possible
fn optimize_chunks(
    mut chunks: Vec<ChunkInfo>,
    target_size: usize,
    min_size: usize,
) -> Vec<ChunkInfo> {
    // First pass: Try to merge small chunks with neighbors
    for i in 0..chunks.len() {
        // Skip chunks that have already been handled
        if chunks[i].1 == 0 || chunks[i].1 >= min_size {
            continue;
        }

        // Try to merge with next chunk
        if i < chunks.len() - 1
            && chunks[i + 1].1 > 0
            && chunks[i + 1].1 + chunks[i].1 <= target_size
        {
            chunks[i + 1].0 .0 = chunks[i].0 .0; // Extend start pointer
            chunks[i + 1].1 += chunks[i].1; // Add token count
            chunks[i].1 = 0; // Mark as merged
            continue;
        }

        // Try to merge with previous chunk
        if i > 0 && chunks[i - 1].1 > 0 && chunks[i - 1].1 + chunks[i].1 <= target_size {
            chunks[i - 1].0 .1 = chunks[i].0 .1; // Extend end pointer
            chunks[i - 1].1 += chunks[i].1; // Add token count
            chunks[i].1 = 0; // Mark as merged
        }
    }

    // Filter out merged chunks
    chunks.into_iter().filter(|&(_, size)| size > 0).collect()
}

/// Creates the final result structure from chunk information
///
/// # Arguments
/// * `md` - The ChunkMd struct with file name
/// * `chunks` - Vector of chunk information
///
/// # Returns
/// * `Resp` - Response structure with formatted chunks
fn create_result(md: &ChunkMd, chunks: Vec<ChunkInfo>) -> Resp {
    Resp {
        result: chunks
            .iter()
            .enumerate()
            .map(|(idx, &((start, end), _))| Chunk {
                id: format!("{}-chunk-{}", md.file_name, idx),
                content: md.md[start..end].to_string(),
            })
            .collect(),
    }
}

#[cfg(target_arch = "wasm32")]
export!(MdChunkerPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md_chunker() {
        let raw = r#"
# Hello, world!
This is a paragraph.

Is this a second paragraph?

## Subheading

## Second Subheading

### Third subheading

| table | column |
|-------|--------|
| row 1, c1 | row 1, c2  |
| row 2, c1 | row 2, c2  |"#;
        let md = ChunkMd {
            file_name: "test.md".to_string(),
            md: raw.to_string(),
            target_chunk_size: Some(40),
            target_min_chunk_size: Some(5),
        };
        let expected_output: serde_json::Value = serde_json::Value::String(
            "# Hello, world!\nThis is a paragraph.\n\nIs this a second paragraph?".to_string(),
        );

        let output = chunk_markdown(&md).unwrap();
        let result = serde_json::to_value(&create_result(&md, output).result[0].content).unwrap();

        assert_eq!(result, expected_output);
    }
}
