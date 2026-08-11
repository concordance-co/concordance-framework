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
use crate::plugin::injector::host::FileType;
use crate::plugin::injector::{
    host::{convert, log, new_client},
    logger::Level,
};

use base64::prelude::*;
use exports::plugin::injector::guest::{Guest, GuestJsonToJson, Metadata, PluginError, PluginKind};

use plugin::injector::host::connect_db;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shared::inlined_schema_for;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct VectorizerRequest {
    pub file_name: String,
    pub convert: ConvertToMd,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ConvertToMd {
    pub file_type: String,
    pub b64_bytes: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ChunkMd {
    pub md: String,
    pub target_chunk_size: usize,
    pub target_min_chunk_size: usize,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Resp {
    pub result: bool,
}

pub struct VectorizerPlugin;

impl Guest for VectorizerPlugin {
    type JsonToJson = Vectorizer;
    fn get_metadata() -> Metadata {
        Metadata {
            name: "Document Vectorizer".to_string(),
            version: "0.1.0".to_string(),
            author: "Brock Elmore".to_string(),
            description: "Converts a document into a vector embedding and stores it in a database."
                .to_string(),
            kind: PluginKind::Tool,
            env_var_support: vec![],
            input_schema: serde_json::to_string(&inlined_schema_for!(VectorizerRequest)).unwrap(),
            default_input: serde_json::to_string(&VectorizerRequest::default()).unwrap(),
            output_schema: serde_json::to_string(&inlined_schema_for!(Resp)).unwrap(),
        }
    }
}
pub struct Vectorizer;
impl GuestJsonToJson for Vectorizer {
    fn work(&self, input: String) -> Result<String, PluginError> {
        // ---- convert ----
        let request: VectorizerRequest =
            serde_json::from_str(&input).map_err(|e| PluginError::Json(e.to_string()))?;
        let file_name = request.file_name;
        let raw_bytes = BASE64_STANDARD
            .decode(request.convert.b64_bytes)
            .map_err(|e| PluginError::MdError(format!("Expected base64 encoded bytes: {}", e)))?;

        let file_type = match request.convert.file_type.as_str() {
            "pdf" => FileType::Pdf(raw_bytes),
            "ppt" => FileType::Ppt(raw_bytes),
            "docx" => FileType::Docx(raw_bytes),
            "xlsx" => FileType::Xlsx(raw_bytes),
            "html" => FileType::Html(raw_bytes),
            _ => return Err(PluginError::MdError("Invalid file type".to_string())),
        };

        let res = convert(&file_type)?;

        let input = ChunkMd {
            md: res,
            target_chunk_size: 1200,
            target_min_chunk_size: 150,
        };

        // ---- chunk ----
        let res = chunk(input);

        // ---- embed & store ----
        let client = new_client("http://127.0.0.1:1234/v1", "lm_studio")?;
        let db = connect_db("test-db")?;

        if !db.get_table_names()?.contains(&"documents".to_string()) {
            let schema_json_str = serde_json::to_string(&arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("id", arrow_schema::DataType::Utf8, false),
                arrow_schema::Field::new("text", arrow_schema::DataType::Utf8, false),
                arrow_schema::Field::new(
                    "embeddings",
                    arrow_schema::DataType::FixedSizeList(
                        std::sync::Arc::new(arrow_schema::Field::new(
                            "item",
                            arrow_schema::DataType::Float32,
                            true,
                        )),
                        768,
                    ),
                    false,
                ),
            ]))
            .unwrap();
            let _ = db.create_table("documents", &schema_json_str)?;
        }

        let ids: Vec<String> = (0..res.len())
            .map(|i| format!("{file_name}-chunk-{i}"))
            .collect::<Vec<String>>();

        let to_add = vec![
            serde_json::to_string(&ids).unwrap(),
            serde_json::to_string(&res).unwrap(),
        ];
        let res = db.add(
            "documents",
            client,
            "text-embedding-nomic-embed-text-v1.5-embedding",
            &to_add,
            1, // embed the chunks column
            Some("id"),
        )?;

        log(Level::Info, "Embeddings created successfully");

        Ok(serde_json::to_string(&Resp { result: res }).unwrap())
    }

    fn new() -> Self {
        Self {}
    }
}

export!(VectorizerPlugin);

fn chunk(md: ChunkMd) -> Vec<String> {
    use markdown::{mdast::Node, to_mdast, ParseOptions};
    use tiktoken_rs::o200k_base;

    let bpe = o200k_base().unwrap();
    let gfm = ParseOptions::gfm();
    let ast = to_mdast(&md.md, &gfm).unwrap();

    let children = ast.children().unwrap();
    let mut sections = vec![];
    let mut current_chunk = (0, 0);
    for child in children {
        // Take until the next heading
        if let Node::Heading(_) = child {
            if current_chunk != (0, 0) {
                sections.push((current_chunk.0, current_chunk.1));
            }
            current_chunk.0 = child.position().unwrap().start.offset;
            current_chunk.1 = child.position().unwrap().end.offset;
        } else {
            current_chunk.1 = child.position().unwrap().end.offset;
        }
    }

    if current_chunk != (0, 0) {
        sections.push((current_chunk.0, current_chunk.1));
    }

    let mut chunks = vec![];
    for section in sections {
        let section_content = &md.md[section.0..section.1];
        let section_tokens = bpe.encode_with_special_tokens(section_content);
        if section_tokens.len() <= md.target_chunk_size {
            chunks.push(((section.0, section.1), section_tokens.len()));
        } else {
            // Implement strategy for chunking larger sections
            let section_nodes = children
                .iter()
                .filter(|node| {
                    if let Some(pos) = node.position() {
                        pos.start.offset >= section.0 && pos.end.offset <= section.1
                    } else {
                        false
                    }
                })
                .collect::<Vec<_>>();

            // Process section_nodes to create smaller chunks
            let mut current_chunk_start = section.0;
            let mut current_chunk_size = 0;

            for node in section_nodes {
                let node_pos = node.position().unwrap();
                let node_content = &md.md[node_pos.start.offset..node_pos.end.offset];
                let node_tokens = bpe.encode_with_special_tokens(node_content);

                // Always add a table
                if let Node::Table(_) = node {
                    current_chunk_size += node_tokens.len();
                    continue;
                }

                // If adding this node would exceed target size, create a chunk
                if current_chunk_size + node_tokens.len() > md.target_chunk_size
                    && current_chunk_size >= md.target_min_chunk_size
                {
                    chunks.push((
                        (current_chunk_start, node_pos.start.offset),
                        current_chunk_size,
                    ));
                    current_chunk_start = node_pos.start.offset;
                    current_chunk_size = node_tokens.len();
                } else {
                    current_chunk_size += node_tokens.len();
                }
            }

            // Add the final chunk if it meets minimum size
            chunks.push(((current_chunk_start, section.1), current_chunk_size));
        }
    }

    for i in 0..chunks.len() {
        if chunks[i].1 < md.target_min_chunk_size {
            // try to add to previous chunk if the previous chunk is not greater
            // than target_chunk_size
            if i > 0 && chunks[i - 1].1 < md.target_chunk_size && chunks[i - 1].1 != 0 {
                // update end ptr
                chunks[i - 1].0 .1 = chunks[i].0 .1;
                // update token count
                chunks[i - 1].1 += chunks[i].1;
                chunks[i].1 = 0;
            } else {
                // try to add to next chunk if the next chunk is not greater
                // than target_chunk_size
                if i < chunks.len() - 1 && chunks[i + 1].1 < md.target_chunk_size {
                    // update start ptr
                    chunks[i + 1].0 .0 = chunks[i].0 .0;
                    // update token count
                    chunks[i + 1].1 += chunks[i].1;
                    chunks[i].1 = 0;
                }
            }
        }
    }
    // Remove chunks that are smaller than the minimum size and were merged
    chunks.retain(|&(_, i)| i != 0);
    chunks
        .iter()
        .map(|((start, end), _)| md.md[*start..*end].to_string())
        .collect()
}
