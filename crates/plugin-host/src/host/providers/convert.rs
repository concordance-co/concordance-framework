use crate::injector::error::FsError;
use crate::injector::markdown_converter::FileType;
use crate::injector::{error, error::PluginError};

/// Converter for transforming various document types to Markdown.
///
/// This struct provides functionality to convert different file types
/// (PDF, PPT, DOCX, XLSX, HTML) to Markdown format using the marker_single
/// command-line tool.
#[derive(Debug, Clone, Default)]
pub struct MdConverter {}

impl MdConverter {
    /// Creates a new instance of the MdConverter.
    ///
    /// # Returns
    ///
    /// A new MdConverter instance.
    pub fn new() -> Self {
        Self {}
    }

    /// Converts a document to Markdown format.
    ///
    /// This method takes a document of a supported type, writes it to a temporary file,
    /// processes it with marker_single, and returns the resulting Markdown content.
    /// Results are cached based on document hash to avoid redundant conversions.
    ///
    /// # Arguments
    ///
    /// * `doc` - The document to convert, wrapped in the appropriate FileType variant
    ///
    /// # Returns
    ///
    /// The Markdown content as a String if successful, or an error if conversion fails
    pub async fn convert(&mut self, doc: FileType) -> Result<String, error::PluginError> {
        // Write to a file based on the doc type
        let (doc_content, extension) = match &doc {
            FileType::Pdf(content) => (content, ".pdf"),
            FileType::Ppt(content) => (content, ".ppt"),
            FileType::Docx(content) => (content, ".docx"),
            FileType::Xlsx(content) => (content, ".xlsx"),
            FileType::Html(content) => (content, ".html"),
        };

        // Calculate a hash of the content for caching
        let hash = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(doc_content);
            // Note that calling `finalize()` consumes hasher
            format!("{:x}", hasher.finalize())
        };

        // Check if we already have processed this file
        let home_dir = match home::home_dir() {
            Some(path) => path,
            None => {
                return Err(PluginError::Fs(FsError::Other(
                    "Could not find home directory".to_string(),
                )))
            }
        };

        let convert_path = home_dir.join(".concordance").join("file-conversions");
        let tmp_convert_path = home_dir
            .join(".concordance")
            .join("file-conversions")
            .join("tmp");
        let tempfile_path = home_dir
            .join(".concordance")
            .join("file-conversions")
            .join("tmp")
            .join(format!("{}{}", &hash, extension));
        let cache_path = convert_path.join(&hash);
        let cache_md_path = cache_path.join(format!("{}.md", &hash));

        // If the markdown file already exists, return it immediately
        if cache_md_path.exists() {
            match std::fs::read_to_string(&cache_md_path) {
                Ok(content) => return Ok(content),
                Err(e) => {
                    return Err(PluginError::Fs(FsError::Other(format!(
                        "Failed to read cached markdown file: {}",
                        e
                    ))))
                }
            }
        }

        // Create a temporary file with the correct extension
        use std::io::Write;
        if !tmp_convert_path.exists() {
            match std::fs::create_dir_all(&tmp_convert_path) {
                Ok(_) => {}
                Err(e) => {
                    return Err(PluginError::Fs(FsError::Other(format!(
                        "Failed to create temp directory: {}",
                        e
                    ))))
                }
            }
        }

        let mut temp_file = match std::fs::File::create(&tempfile_path) {
            Ok(file) => file,
            Err(e) => {
                return Err(PluginError::Fs(FsError::Other(format!(
                    "Failed to create temp file: {}",
                    e
                ))))
            }
        };

        match temp_file.write_all(doc_content) {
            Ok(_) => {}
            Err(e) => {
                return Err(PluginError::Fs(FsError::Other(format!(
                    "Failed to write to temp file: {}",
                    e
                ))))
            }
        }

        // Call marker_single command
        // Ensure the output directory exists
        if !convert_path.exists() {
            match std::fs::create_dir_all(&convert_path) {
                Ok(_) => {}
                Err(e) => {
                    return Err(PluginError::Fs(FsError::Other(format!(
                        "Failed to create output directory: {}",
                        e
                    ))))
                }
            }
        }

        let convert_path_str = convert_path.to_string_lossy().to_string();

        println!("running marker_single, output dir: {}", convert_path_str);
        let output = match std::process::Command::new("marker_single")
            .arg(tempfile_path)
            .arg("--output_dir")
            .arg(convert_path_str)
            .output()
        {
            Ok(output) => output,
            Err(e) => {
                return Err(PluginError::MdError(format!(
                    "Failed to execute marker_single: {}",
                    e
                )))
            }
        };

        // Check if the command executed successfully
        if !output.status.success() {
            let error_message = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(PluginError::MdError(format!(
                "marker_single failed: {}",
                error_message
            )));
        }

        println!(
            "marker_single output: {:#?}",
            String::from_utf8_lossy(&output.stdout[..])
        );

        // Read the markdown file
        let markdown_content = match std::fs::read_to_string(&cache_md_path) {
            Ok(content) => content,
            Err(e) => {
                return Err(PluginError::Fs(FsError::Other(format!(
                    "Failed to read markdown file at {}: {}",
                    cache_md_path.to_string_lossy(),
                    e
                ))))
            }
        };

        Ok(markdown_content)
    }
}
