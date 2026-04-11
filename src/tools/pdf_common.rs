//! Common PDF extraction functions shared between PDF tools
//!
//! This module provides shared functionality for:
//! - Image extraction from PDFs (lopdf, pdfium)
//! - Page selection and parsing
//! - Image encoding utilities

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use lopdf::Document;
use std::path::{Path, PathBuf};

/// Minimum character threshold to consider text extraction successful
pub const MIN_TEXT_CHARS: usize = 50;

/// Minimum image size to consider for OCR (skip tiny icons)
pub const MIN_IMAGE_SIZE: usize = 10000;

/// Information about an extracted image
pub struct ExtractedImage {
    pub page_num: u32,
    pub data: Vec<u8>,
    pub format: ImageFormat,
}

/// Supported image formats
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Raw,
}

/// Resolve a path relative to the working directory
pub fn resolve_path(working_dir: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(working_dir).join(path)
    }
}

// ============================================================================
// LOPDF - Pure Rust image extraction (for scanned PDFs with embedded images)
// ============================================================================

/// Extract embedded images from a PDF using lopdf (pure Rust)
pub fn extract_images_lopdf(path: &Path) -> Result<Vec<ExtractedImage>> {
    let doc = Document::load(path)?;
    let mut images = Vec::new();

    for (page_num, _page_id) in doc.get_pages() {
        for object_id in doc.objects.keys() {
            if let Ok(object) = doc.get_object(*object_id) {
                if let Ok(stream) = object.as_stream() {
                    let dict = &stream.dict;

                    let is_image = dict
                        .get(b"Subtype")
                        .ok()
                        .and_then(|s| s.as_name().ok())
                        .map(|n| n == b"Image")
                        .unwrap_or(false);

                    if !is_image {
                        continue;
                    }

                    let filter: Option<Vec<u8>> = dict
                        .get(b"Filter")
                        .ok()
                        .and_then(|f| {
                            if let Ok(name) = f.as_name() {
                                Some(name.to_vec())
                            } else if let Ok(arr) = f.as_array() {
                                arr.first()
                                    .and_then(|f| f.as_name().ok())
                                    .map(|n| n.to_vec())
                            } else {
                                None
                            }
                        });

                    let format = match filter.as_deref() {
                        Some(b"DCTDecode") => ImageFormat::Jpeg,
                        Some(b"FlateDecode") => ImageFormat::Png,
                        Some(b"JPXDecode") => ImageFormat::Jpeg,
                        _ => ImageFormat::Raw,
                    };

                    let data = if format == ImageFormat::Jpeg {
                        stream.content.clone()
                    } else {
                        stream.decompressed_content().unwrap_or_else(|_| stream.content.clone())
                    };

                    if data.len() > MIN_IMAGE_SIZE {
                        images.push(ExtractedImage {
                            page_num,
                            data,
                            format,
                        });
                    }
                }
            }
        }
    }

    images.dedup_by(|a, b| a.data == b.data);
    images.sort_by_key(|img| img.page_num);

    Ok(images)
}

// ============================================================================
// PDFIUM - Optional rendering for vector PDFs (requires pdfium.dll)
// ============================================================================

/// Render PDF pages to images using pdfium
#[cfg(feature = "pdfium")]
pub fn render_pages_pdfium(path: &Path) -> Result<Vec<ExtractedImage>> {
    use pdfium_render::prelude::*;

    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
            .or_else(|_| Pdfium::bind_to_system_library())
            .map_err(|e| anyhow::anyhow!(
                "pdfium.dll not found!\n\n\
                 Download from: https://github.com/bblanchon/pdfium-binaries/releases\n\
                 Extract pdfium.dll from bin/ to the application directory.\n\n\
                 Error: {:?}", e
            ))?
    );

    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    let page_count = document.pages().len();
    tracing::info!("pdfium: Rendering {} pages to images", page_count);

    let mut images = Vec::with_capacity(page_count as usize);

    for i in 0..page_count {
        let page = document
            .pages()
            .get(i)
            .map_err(|e| anyhow::anyhow!("Failed to get page {}: {:?}", i, e))?;

        let render_config = PdfRenderConfig::new()
            .set_target_width(1200)
            .set_maximum_height(1600);

        let bitmap = page
            .render_with_config(&render_config)
            .map_err(|e| anyhow::anyhow!("Failed to render page {}: {:?}", i, e))?;

        let img = bitmap.as_image();

        let mut buffer = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buffer);
        img.write_to(&mut cursor, image::ImageFormat::Jpeg)
            .map_err(|e| anyhow::anyhow!("Failed to encode page {}: {}", i, e))?;

        images.push(ExtractedImage {
            page_num: (i + 1) as u32,
            data: buffer,
            format: ImageFormat::Jpeg,
        });
    }

    Ok(images)
}

#[cfg(not(feature = "pdfium"))]
pub fn render_pages_pdfium(_path: &Path) -> Result<Vec<ExtractedImage>> {
    Err(anyhow::anyhow!(
        "pdfium feature not enabled.\n\n\
         To enable pdfium for vector PDF support:\n\
         1. Rebuild with: cargo build --features pdfium\n\
         2. Download pdfium.dll from:\n\
            https://github.com/bblanchon/pdfium-binaries/releases\n\
         3. Extract pdfium.dll from bin/ folder to the application directory"
    ))
}

/// Check if pdfium is available at runtime
#[cfg(feature = "pdfium")]
pub fn is_pdfium_available() -> bool {
    use pdfium_render::prelude::*;
    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
        .or_else(|_| Pdfium::bind_to_system_library())
        .is_ok()
}

#[cfg(not(feature = "pdfium"))]
pub fn is_pdfium_available() -> bool {
    false
}

// ============================================================================
// Helper functions
// ============================================================================

/// Convert an extracted image to base64
pub fn image_to_base64(image: &ExtractedImage) -> Result<String> {
    match image.format {
        ImageFormat::Jpeg => Ok(BASE64.encode(&image.data)),
        ImageFormat::Png | ImageFormat::Raw => {
            if let Ok(img) = image::load_from_memory(&image.data) {
                let mut buffer = Vec::new();
                let mut cursor = std::io::Cursor::new(&mut buffer);
                img.write_to(&mut cursor, image::ImageFormat::Png)?;
                Ok(BASE64.encode(&buffer))
            } else {
                Ok(BASE64.encode(&image.data))
            }
        }
    }
}

/// Get MIME type for an image format
pub fn get_mime_type(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png | ImageFormat::Raw => "image/png",
    }
}

/// Get PDF page count using lopdf (fallback)
pub fn get_pdf_page_count_lopdf(path: &Path) -> Result<u32> {
    let doc = Document::load(path)?;
    Ok(doc.get_pages().len() as u32)
}

/// Get PDF page count using pdfium (more reliable)
#[cfg(feature = "pdfium")]
pub fn get_pdf_page_count_pdfium(path: &Path) -> Result<u32> {
    use pdfium_render::prelude::*;

    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
            .or_else(|_| Pdfium::bind_to_system_library())
            .map_err(|e| anyhow::anyhow!("pdfium not available: {:?}", e))?
    );

    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("Failed to load PDF: {:?}", e))?;

    Ok(document.pages().len() as u32)
}

#[cfg(not(feature = "pdfium"))]
pub fn get_pdf_page_count_pdfium(path: &Path) -> Result<u32> {
    get_pdf_page_count_lopdf(path)
}

/// Get PDF page count - uses pdfium if available, lopdf as fallback
pub fn get_pdf_page_count(path: &Path) -> Result<u32> {
    if is_pdfium_available() {
        match get_pdf_page_count_pdfium(path) {
            Ok(count) => return Ok(count),
            Err(e) => {
                tracing::warn!("pdfium page count failed, falling back to lopdf: {}", e);
            }
        }
    }
    get_pdf_page_count_lopdf(path)
}

// ============================================================================
// Page selection
// ============================================================================

/// Parse page ranges from a spec string like "1-5,8,10-12"
fn parse_page_ranges(spec: &str, total_pages: u32) -> Result<Vec<u32>> {
    let mut pages = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if part.contains('-') {
            let bounds: Vec<&str> = part.split('-').collect();
            if bounds.len() != 2 {
                return Err(anyhow::anyhow!("Invalid range format: {}", part));
            }
            let start: u32 = bounds[0].trim().parse()
                .map_err(|_| anyhow::anyhow!("Invalid page number: {}", bounds[0]))?;
            let end: u32 = bounds[1].trim().parse()
                .map_err(|_| anyhow::anyhow!("Invalid page number: {}", bounds[1]))?;
            if start > end {
                return Err(anyhow::anyhow!("Invalid range: {} > {}", start, end));
            }
            pages.extend(start..=end);
        } else {
            let page: u32 = part.parse()
                .map_err(|_| anyhow::anyhow!("Invalid page number: {}", part))?;
            pages.push(page);
        }
    }
    pages.retain(|&p| p >= 1 && p <= total_pages);
    pages.sort();
    pages.dedup();
    Ok(pages)
}

/// Parse page selection string into a list of page numbers
/// Formats: "8-9", "1,3,5", "1-5,10", "!3-5" (exclude)
pub fn parse_pages(spec: &str, total_pages: u32) -> Result<Vec<u32>> {
    let spec = spec.trim();

    // Exclusion mode: "!3-5" means all except 3-5
    if let Some(rest) = spec.strip_prefix('!') {
        let exclude = parse_page_ranges(rest, total_pages)?;
        let all: Vec<u32> = (1..=total_pages).collect();
        return Ok(all.into_iter().filter(|p| !exclude.contains(p)).collect());
    }

    parse_page_ranges(spec, total_pages)
}

/// Format a list of pages into a compact string (e.g., [1,2,3,5,6,8] -> "1-3,5-6,8")
pub fn format_pages_compact(pages: &[u32]) -> String {
    if pages.is_empty() {
        return String::new();
    }

    let mut result = Vec::new();
    let mut range_start = pages[0];
    let mut range_end = pages[0];

    for &page in pages.iter().skip(1) {
        if page == range_end + 1 {
            range_end = page;
        } else {
            if range_start == range_end {
                result.push(format!("{}", range_start));
            } else {
                result.push(format!("{}-{}", range_start, range_end));
            }
            range_start = page;
            range_end = page;
        }
    }

    // Don't forget the last range
    if range_start == range_end {
        result.push(format!("{}", range_start));
    } else {
        result.push(format!("{}-{}", range_start, range_end));
    }

    result.join(",")
}

// ============================================================================
// Output format support
// ============================================================================

/// Output format for PDF extraction
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum OutputFormat {
    #[default]
    Txt,
    Csv,
}

impl OutputFormat {
    /// Parse format from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "txt" | "text" => Some(OutputFormat::Txt),
            "csv" => Some(OutputFormat::Csv),
            _ => None,
        }
    }

    /// Get file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            OutputFormat::Txt => "txt",
            OutputFormat::Csv => "csv",
        }
    }
}

/// Default CSV separator (semicolon for French Excel)
pub const DEFAULT_CSV_SEPARATOR: char = ';';

/// OCR prompt for TXT format (full text extraction)
pub const OCR_PROMPT_TXT: &str = r#"Extract ALL text from the image exactly as it appears.
Preserve the original layout, line breaks, and formatting as much as possible.
Include all visible text: headers, paragraphs, tables, footnotes, page numbers.
Do not summarize or interpret - just transcribe verbatim.
Output ONLY the extracted text, no explanations."#;

/// OCR prompt for CSV format (structured table extraction)
pub fn ocr_prompt_csv(separator: char) -> String {
    format!(r#"Extract this CERFA table as CSV. Use '{sep}' separator.
Each row has: CODE (2 letters like AA, AB, AC){sep}BRUT value{sep}AMORT value{sep}NET value
Read the code from the left margin (AA, AB, AC, AD...). Output one line per code.
Example output:
AB{sep}2000{sep}{sep}2000
AC{sep}{sep}500{sep}
Output data only:"#, sep = separator)
}

/// Clean and validate CSV output
pub fn postprocess_csv(raw: &str, separator: char) -> String {
    let sep_str = separator.to_string();
    let mut lines: Vec<String> = Vec::new();

    // First, try to remove thinking blocks that may be in the output
    let cleaned = remove_thinking_blocks(raw);

    for line in cleaned.lines() {
        let trimmed = line.trim();

        // Skip empty lines and common artifacts
        if trimmed.is_empty()
            || trimmed.starts_with("```")
            || trimmed.starts_with('#')
            || trimmed.starts_with('-')
            || trimmed.starts_with("*")
            || trimmed.to_lowercase().starts_with("here")
            || trimmed.to_lowercase().starts_with("note:")
            || trimmed.to_lowercase().starts_with("output")
            || trimmed.to_lowercase().starts_with("csv")
            || trimmed.to_lowercase().contains("separator")
            || trimmed.to_lowercase().contains("column")
            || trimmed.to_lowercase().contains("format")
            || trimmed.contains("...")
        {
            continue;
        }

        // Normalize separators: replace tabs and commas with the chosen separator
        let normalized = if separator != ',' && trimmed.contains(',') && !trimmed.contains(separator) {
            trimmed.replace(',', &sep_str)
        } else if separator != '\t' && trimmed.contains('\t') && !trimmed.contains(separator) {
            trimmed.replace('\t', &sep_str)
        } else {
            trimmed.to_string()
        };

        // Only keep lines that look like CSV data (contain separator)
        if normalized.contains(separator) {
            // Additional validation: should have multiple fields
            let field_count = normalized.matches(separator).count() + 1;
            if field_count >= 2 {
                lines.push(normalized);
            }
        }
    }

    lines.join("\n")
}

/// Remove thinking blocks from model output
fn remove_thinking_blocks(text: &str) -> String {
    // Simple approach: find <think> and remove everything from there
    // This handles both closed </think> and truncated responses
    if let Some(start) = text.find("<think>") {
        if let Some(end) = text.find("</think>") {
            // Closed block: remove it and continue with rest
            let before = &text[..start];
            let after = &text[end + 8..]; // 8 = "</think>".len()
            return format!("{}{}", before.trim(), remove_thinking_blocks(after));
        } else {
            // Unclosed block: keep only what's before
            return text[..start].trim().to_string();
        }
    }
    text.to_string()
}

/// Interactive prompt for output format selection
pub fn prompt_output_format() -> Result<OutputFormat> {
    use crate::cli::tool_prompts::tool_prompt_choice;

    let choice = tool_prompt_choice(
        "Format de sortie",
        &["TXT - Texte brut", "CSV - Tableau (Excel)"]
    ).map_err(|e| anyhow::anyhow!("Input error: {}", e))?;

    match choice {
        1 => Ok(OutputFormat::Txt),
        2 => Ok(OutputFormat::Csv),
        _ => Ok(OutputFormat::Txt),
    }
}
