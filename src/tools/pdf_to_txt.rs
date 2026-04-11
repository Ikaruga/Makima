//! PDF to TXT/CSV extraction tool with OCR fallback for scanned documents
//!
//! Strategy:
//! 1. Try native text extraction (pdf_extract)
//! 2. If insufficient text, try extracting embedded images (lopdf - pure Rust)
//! 3. If no images found and pdfium feature enabled, render pages to images (pdfium)
//! 4. Send images to vision model for OCR
//!
//! Supports output formats:
//! - TXT: Plain text extraction (default)
//! - CSV: Structured table extraction with semicolon separator (French Excel compatible)

use super::pdf_common::{
    ExtractedImage, OutputFormat,
    DEFAULT_CSV_SEPARATOR, MIN_TEXT_CHARS,
    extract_images_lopdf, render_pages_pdfium, is_pdfium_available,
    image_to_base64, get_mime_type, resolve_path,
    parse_pages, format_pages_compact, get_pdf_page_count,
    prompt_output_format, ocr_prompt_csv, postprocess_csv,
    OCR_PROMPT_TXT,
};
use super::registry::{Tool, ToolResult};
use crate::llm::client::LmStudioClient;
use crate::llm::types::ParsedToolCall;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Seuil de pages pour proposer la comparaison OCR vs natif
const COMPARE_THRESHOLD: u32 = 20;

/// Tool for extracting text from PDF files
pub struct PdfToTxtTool {
    working_dir: String,
    client: Option<Arc<LmStudioClient>>,
}

impl PdfToTxtTool {
    pub fn new(working_dir: String) -> Self {
        Self {
            working_dir,
            client: None,
        }
    }

    /// Create with an LM Studio client for OCR fallback
    pub fn with_client(working_dir: String, client: Arc<LmStudioClient>) -> Self {
        Self {
            working_dir,
            client: Some(client),
        }
    }

    /// Try native text extraction
    fn extract_native_text(&self, path: &Path) -> Result<String, String> {
        pdf_extract::extract_text(path).map_err(|e| e.to_string())
    }
}


/// Interactive prompt for page selection when pages parameter is not provided
fn prompt_page_selection(total_pages: u32) -> Result<Option<Vec<u32>>> {
    use crate::cli::tool_prompts::{tool_prompt_choice, tool_prompt_text};

    let status = format!("PDF: {} pages", total_pages);
    let choice = tool_prompt_choice(&status, &["Tout", "Selection", "Exclure"])
        .map_err(|e| anyhow::anyhow!("Input error: {}", e))?;

    match choice {
        1 => Ok(None), // Tout
        2 => {
            let input = tool_prompt_text("Pages (ex: 1-5,8,10):")?;
            if input.is_empty() {
                Ok(None)
            } else {
                Ok(Some(parse_pages(&input, total_pages)?))
            }
        }
        3 => {
            let input = tool_prompt_text("Exclure (ex: 3-5):")?;
            if input.is_empty() {
                Ok(None)
            } else {
                Ok(Some(parse_pages(&format!("!{}", input), total_pages)?))
            }
        }
        _ => Ok(None),
    }
}

#[async_trait]
impl Tool for PdfToTxtTool {
    fn name(&self) -> &str {
        "pdf_to_txt"
    }

    fn description(&self) -> &str {
        "Extraire le texte d'un PDF via OCR vision. Supporte TXT (texte brut) et CSV (tableau Excel)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Chemin du fichier PDF"
                },
                "output": {
                    "type": "string",
                    "description": "Chemin du fichier de sortie (optionnel, extension auto selon format)"
                },
                "format": {
                    "type": "string",
                    "enum": ["txt", "csv"],
                    "description": "Format de sortie: 'txt' (texte brut, defaut) ou 'csv' (tableau Excel)"
                },
                "separator": {
                    "type": "string",
                    "description": "Separateur CSV (defaut: ';' pour Excel francais)"
                },
                "force_ocr": {
                    "type": "boolean",
                    "description": "Utiliser l'OCR (defaut: true). Mettre a false pour extraction native."
                },
                "use_pdfium": {
                    "type": "boolean",
                    "description": "Forcer le rendu pdfium pour PDF vectoriels (necessite pdfium.dll)"
                },
                "pages": {
                    "type": "string",
                    "description": "Pages a extraire: '8-9', '1,3,5', '!3-5' (exclure). Si absent, demande a l'utilisateur."
                },
                "test": {
                    "type": "boolean",
                    "description": "Mode test: extrait 3 pages echantillon pour estimer le temps total (defaut: false)"
                }
            },
            "required": ["input"]
        })
    }

    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult> {
        let input = args
            .get_string("input")
            .ok_or_else(|| anyhow::anyhow!("Missing 'input' parameter"))?;

        let force_ocr = args.get_bool("force_ocr").unwrap_or(true);  // OCR by default
        let use_pdfium = args.get_bool("use_pdfium");
        let pages_param = args.get_string("pages");
        let test_mode = args.get_bool("test").unwrap_or(false);
        let input_path = resolve_path(&self.working_dir, &input);

        // Parse format parameter or prompt user
        let format_param = args.get_string("format");
        let output_format = if let Some(fmt) = format_param {
            OutputFormat::from_str(&fmt).unwrap_or(OutputFormat::Txt)
        } else {
            // Interactive format selection
            prompt_output_format()?
        };

        // Parse CSV separator
        let separator = args.get_string("separator")
            .and_then(|s| s.chars().next())
            .unwrap_or(DEFAULT_CSV_SEPARATOR);

        // Validate file exists early
        if !input_path.exists() {
            return Ok(ToolResult::error(format!("PDF not found: {}", input_path.display())));
        }

        let ext = input_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext.to_lowercase() != "pdf" {
            return Ok(ToolResult::error(format!("Not a PDF: {}", input_path.display())));
        }

        // Test mode: extract sample pages and estimate time
        if test_mode {
            return self.perform_test(&input_path, use_pdfium, output_format, separator).await;
        }

        let output = args.get_string("output").unwrap_or_else(|| {
            let stem = input_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
            let parent = input_path.parent().unwrap_or(Path::new("."));
            parent.join(format!("{}.{}", stem, output_format.extension())).to_string_lossy().to_string()
        });
        let output_path = resolve_path(&self.working_dir, &output);

        // Detecter automatiquement la meilleure methode d'extraction
        let total_pages = get_pdf_page_count(&input_path)?;
        let use_ocr = if self.client.is_some() && pages_param.is_none() {
            use crate::cli::tool_prompts::{tool_clear, tool_status};

            tool_status(&format!("Detection du type de PDF ({} pages)...", total_pages));

            // 1. Tester si le PDF contient des images (= scan)
            let path_clone = input_path.clone();
            let has_images = tokio::task::spawn_blocking(move || {
                extract_images_lopdf(&path_clone)
                    .map(|imgs| !imgs.is_empty())
                    .unwrap_or(false)
            }).await.unwrap_or(false);

            // 2. Tester l'extraction native sur page 1
            let native_text = self.extract_native_text(&input_path).unwrap_or_default();
            let native_chars = native_text.trim().len();
            let has_native_text = native_chars >= MIN_TEXT_CHARS;

            // 3. Recommander automatiquement
            let (recommendation, default_ocr) = if has_images && !has_native_text {
                ("PDF scanne detecte (images, pas de texte natif) → OCR recommande", true)
            } else if has_native_text && !has_images {
                ("PDF vectoriel detecte (texte natif present) → Natif recommande", false)
            } else if has_native_text && has_images {
                ("PDF mixte detecte (texte natif + images) → Comparaison recommandee", true)
            } else {
                ("Aucun texte ni image detecte → OCR recommande", true)
            };

            tool_status(recommendation);

            // 4. Proposer le choix a l'utilisateur avec la recommandation
            if total_pages > COMPARE_THRESHOLD {
                match self.prompt_method_choice(recommendation, default_ocr, has_native_text, native_chars) {
                    Ok(choice) => {
                        tool_clear();
                        choice
                    }
                    Err(_) => {
                        tool_clear();
                        default_ocr
                    }
                }
            } else {
                tool_clear();
                default_ocr
            }
        } else {
            force_ocr
        };

        let (text, method, pages_info) = if use_ocr {
            if self.client.is_none() {
                return Ok(ToolResult::error("OCR requires vision client (LM Studio)"));
            }
            match self.perform_ocr(&input_path, use_pdfium, pages_param.as_deref(), output_format, separator).await {
                Ok((t, m, p)) => (t, format!("OCR ({})", m), p),
                Err(e) => return Ok(ToolResult::error(format!("OCR failed: {}", e))),
            }
        } else {
            match self.extract_native_text(&input_path) {
                Ok(text) if text.trim().len() >= MIN_TEXT_CHARS => {
                    (text, "native".to_string(), None)
                }
                _ => {
                    if self.client.is_none() {
                        return Ok(ToolResult::error(
                            "PDF has no extractable text and OCR requires vision client"
                        ));
                    }
                    tracing::info!("Native extraction failed, trying OCR...");
                    match self.perform_ocr(&input_path, use_pdfium, pages_param.as_deref(), output_format, separator).await {
                        Ok((t, m, p)) => (t, format!("OCR ({})", m), p),
                        Err(e) => return Ok(ToolResult::error(format!("OCR failed: {}", e))),
                    }
                }
            }
        };

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output_path, &text)?;

        let pages_str = pages_info
            .map(|p| format!(" (pages: {})", p))
            .unwrap_or_default();

        let format_str = match output_format {
            OutputFormat::Txt => "TXT",
            OutputFormat::Csv => "CSV",
        };

        Ok(ToolResult::success(format!(
            "Extracted {} chars, {} lines as {} using {}{} -> {}",
            text.len(),
            text.lines().count(),
            format_str,
            method,
            pages_str,
            output_path.display()
        )))
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn summarize_call(&self, args: &ParsedToolCall) -> String {
        let input = args.get_string("input").unwrap_or_else(|| "?".to_string());
        let test_mode = args.get_bool("test").unwrap_or(false);
        if test_mode {
            return format!("Test extraction from {} (3 sample pages)", input);
        }
        let format = args.get_string("format")
            .map(|f| format!(" [{}]", f.to_uppercase()))
            .unwrap_or_default();
        let force = if args.get_bool("force_ocr").unwrap_or(false) { " (OCR)" } else { "" };
        let pages = args.get_string("pages")
            .map(|p| format!(" [pages: {}]", p))
            .unwrap_or_default();
        format!("Extract{} from {}{}{}", format, input, force, pages)
    }
}

impl PdfToTxtTool {
    /// Perform OCR using the best available method
    /// Returns (text, method, pages_info)
    async fn perform_ocr(
        &self,
        path: &Path,
        use_pdfium: Option<bool>,
        pages_param: Option<&str>,
        output_format: OutputFormat,
        separator: char,
    ) -> Result<(String, String, Option<String>)> {
        let client = self.client.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Vision client not available"))?
            .clone();

        // Get the real PDF page count (not derived from extracted images)
        let path_for_count = path.to_path_buf();
        let pdf_page_count = tokio::task::spawn_blocking(move || get_pdf_page_count(&path_for_count))
            .await??;
        tracing::info!("PDF has {} pages", pdf_page_count);

        // Step 1: Try pdfium FIRST (renders every page - best for vector PDFs)
        let should_use_pdfium = use_pdfium.unwrap_or_else(is_pdfium_available);

        if should_use_pdfium {
            tracing::info!("Trying pdfium rendering...");
            let path_clone = path.to_path_buf();
            match tokio::task::spawn_blocking(move || render_pages_pdfium(&path_clone)).await? {
                Ok(images) if !images.is_empty() => {
                    tracing::info!("pdfium: Rendered {} pages", images.len());
                    let (filtered_images, pages_info) = self.filter_by_pages(images, pages_param, pdf_page_count)?;
                    let text = self.ocr_images(&client, &filtered_images, output_format, separator).await?;
                    return Ok((text, "pdfium".to_string(), pages_info));
                }
                Ok(_) => {
                    tracing::warn!("pdfium rendered 0 pages");
                }
                Err(e) => {
                    tracing::warn!("pdfium failed: {}", e);
                }
            }
        }

        // Step 2: Fallback to lopdf (pure Rust) - for scanned PDFs with embedded images
        tracing::info!("Trying lopdf image extraction...");
        let path_clone = path.to_path_buf();
        let images = tokio::task::spawn_blocking(move || extract_images_lopdf(&path_clone))
            .await??;

        if !images.is_empty() {
            tracing::info!("lopdf: Found {} embedded images", images.len());
            let (filtered_images, pages_info) = self.filter_by_pages(images, pages_param, pdf_page_count)?;
            let text = self.ocr_images(&client, &filtered_images, output_format, separator).await?;
            return Ok((text, "lopdf".to_string(), pages_info));
        }

        // No method worked
        let hint = if cfg!(feature = "pdfium") {
            if is_pdfium_available() {
                "pdfium available but no pages rendered"
            } else {
                "pdfium.dll not found - download from github.com/bblanchon/pdfium-binaries/releases"
            }
        } else {
            "For vector PDFs, rebuild with: cargo build --features pdfium"
        };

        Err(anyhow::anyhow!("No images found in PDF. {}", hint))
    }

    /// Filter images by page selection (from parameter or interactive prompt)
    /// `pdf_page_count`: le vrai nombre de pages du PDF (pas le nombre d'images extraites)
    fn filter_by_pages(&self, images: Vec<ExtractedImage>, pages_param: Option<&str>, pdf_page_count: u32) -> Result<(Vec<ExtractedImage>, Option<String>)> {
        let total_pages = pdf_page_count;

        // Determine which pages to extract
        let selected_pages = if let Some(pages_spec) = pages_param {
            Some(parse_pages(pages_spec, total_pages)?)
        } else {
            prompt_page_selection(total_pages)?
        };

        // Filter images based on selection
        let (filtered_images, pages_info) = if let Some(ref pages) = selected_pages {
            let filtered: Vec<_> = images.into_iter()
                .filter(|img| pages.contains(&img.page_num))
                .collect();

            // Build a compact string representation of selected pages
            let pages_str = format_pages_compact(pages);
            (filtered, Some(pages_str))
        } else {
            (images, None)
        };

        if filtered_images.is_empty() {
            return Err(anyhow::anyhow!("No pages selected or found matching the selection"));
        }

        Ok((filtered_images, pages_info))
    }

    /// OCR a list of images using the vision model
    async fn ocr_images(
        &self,
        client: &LmStudioClient,
        images: &[ExtractedImage],
        output_format: OutputFormat,
        separator: char,
    ) -> Result<String> {
        use crate::cli::tool_prompts::{tool_done, tool_progress};

        let mut all_text = Vec::new();
        let total = images.len();
        let start_time = Instant::now();

        // Select prompt based on output format
        let prompt = match output_format {
            OutputFormat::Txt => OCR_PROMPT_TXT.to_string(),
            OutputFormat::Csv => ocr_prompt_csv(separator),
        };

        for (i, image) in images.iter().enumerate() {
            let elapsed = start_time.elapsed();

            // Display progress in tool zone
            let format_label = match output_format {
                OutputFormat::Txt => "OCR TXT",
                OutputFormat::Csv => "OCR CSV",
            };
            tool_progress(format_label, i + 1, total, elapsed.as_secs_f64());

            let b64 = image_to_base64(image)?;
            let mime = get_mime_type(image.format);
            let text = client.ocr_image_with_prompt(&b64, mime, &prompt).await?;

            if !text.trim().is_empty() {
                match output_format {
                    OutputFormat::Txt => {
                        all_text.push(format!("--- Page {} ---\n{}", image.page_num, text));
                    }
                    OutputFormat::Csv => {
                        // For CSV, post-process each page and don't add page markers
                        let cleaned = postprocess_csv(&text, separator);
                        if !cleaned.is_empty() {
                            all_text.push(cleaned);
                        }
                    }
                }
            }
        }

        // Final message
        let elapsed = start_time.elapsed();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;
        let format_str = match output_format {
            OutputFormat::Txt => "TXT",
            OutputFormat::Csv => "CSV",
        };
        tool_done(&format!("OCR {} termine - {} pages en {:02}:{:02}", format_str, total, mins, secs));

        if all_text.is_empty() {
            return Err(anyhow::anyhow!("OCR produced no text"));
        }

        // Join based on format
        let result = match output_format {
            OutputFormat::Txt => all_text.join("\n\n"),
            OutputFormat::Csv => all_text.join("\n"), // No extra blank lines for CSV
        };

        Ok(result)
    }

    /// Perform test extraction on sample pages to estimate total time
    async fn perform_test(
        &self,
        path: &Path,
        use_pdfium: Option<bool>,
        output_format: OutputFormat,
        separator: char,
    ) -> Result<ToolResult> {
        // Verify client is available
        if self.client.is_none() {
            return Ok(ToolResult::error("Client vision requis pour le mode test"));
        }

        // 1. Count total pages
        let total_pages = get_pdf_page_count(path)?;

        if total_pages == 0 {
            return Ok(ToolResult::error("PDF has 0 pages"));
        }

        // 2. Select sample pages: first, middle, last
        let sample_pages: Vec<u32> = if total_pages <= 3 {
            (1..=total_pages).collect()
        } else {
            vec![1, total_pages / 2, total_pages]
        };

        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.pdf");

        let format_str = match output_format {
            OutputFormat::Txt => "TXT",
            OutputFormat::Csv => "CSV",
        };

        eprintln!("\nTEST D'EXTRACTION {} - {}\n", format_str, filename);
        eprintln!("Pages totales: {}", total_pages);
        eprintln!("Pages testees: {} ({:?})\n", sample_pages.len(), sample_pages);
        eprintln!("Resultats:");

        // 3. Extract and time each sample page
        let mut times: Vec<Duration> = Vec::new();
        let mut results: Vec<(u32, Duration, usize)> = Vec::new();
        let mut errors: Vec<(u32, String)> = Vec::new();

        for &page_num in &sample_pages {
            let start = Instant::now();

            // Extract single page using perform_ocr
            let page_spec = page_num.to_string();
            match self.perform_ocr(path, use_pdfium, Some(&page_spec), output_format, separator).await {
                Ok((text, _method, _)) => {
                    let elapsed = start.elapsed();
                    let char_count = text.len();
                    times.push(elapsed);
                    results.push((page_num, elapsed, char_count));

                    eprintln!(
                        "  Page {:>3} : {:.1}s - {} caracteres extraits",
                        page_num,
                        elapsed.as_secs_f64(),
                        char_count
                    );
                }
                Err(e) => {
                    let elapsed = start.elapsed();
                    errors.push((page_num, e.to_string()));
                    eprintln!(
                        "  Page {:>3} : {:.1}s - ERREUR: {}",
                        page_num,
                        elapsed.as_secs_f64(),
                        e
                    );
                }
            }
        }

        // 4. Calculate estimates
        if times.is_empty() {
            return Ok(ToolResult::error(format!(
                "Test echoue: aucune page n'a pu etre extraite.\nErreurs: {:?}",
                errors
            )));
        }

        let avg_time_secs = times.iter().map(|t| t.as_secs_f64()).sum::<f64>() / times.len() as f64;
        let total_estimate_secs = avg_time_secs * total_pages as f64;
        let total_estimate_mins = total_estimate_secs / 60.0;

        let avg_chars = if !results.is_empty() {
            results.iter().map(|(_, _, c)| *c).sum::<usize>() / results.len()
        } else {
            0
        };

        // 5. Format report
        eprintln!("\nEstimation:");
        eprintln!("  Temps moyen/page : {:.1}s", avg_time_secs);

        if total_estimate_mins < 1.0 {
            eprintln!("  Temps total estime: ~{:.0}s ({} pages)", total_estimate_secs, total_pages);
        } else {
            eprintln!("  Temps total estime: ~{:.0} minutes ({} pages)", total_estimate_mins, total_pages);
        }

        // 6. Recommendation
        let recommendation = if errors.is_empty() && avg_chars > 100 {
            "\nRecommandation: Extraction possible"
        } else if !errors.is_empty() {
            "\nRecommandation: Certaines pages ont echoue, verifiez le PDF"
        } else {
            "\nRecommandation: Peu de texte extrait, resultats potentiellement limites"
        };
        eprintln!("{}", recommendation);

        // Build summary for tool result
        let summary = format!(
            "Test {} termine: {} pages testees sur {}\n\
             Temps moyen: {:.1}s/page\n\
             Temps total estime: {}\n\
             Caracteres moyens/page: {}\n\
             Erreurs: {}",
            format_str,
            results.len(),
            total_pages,
            avg_time_secs,
            if total_estimate_mins < 1.0 {
                format!("~{:.0} secondes", total_estimate_secs)
            } else {
                format!("~{:.0} minutes", total_estimate_mins)
            },
            avg_chars,
            if errors.is_empty() { "aucune".to_string() } else { format!("{}", errors.len()) }
        );

        Ok(ToolResult::success(summary))
    }

    /// Demander a l'utilisateur quelle methode utiliser apres detection automatique
    fn prompt_method_choice(&self, _recommendation: &str, default_ocr: bool, has_native: bool, native_chars: usize) -> Result<bool> {
        // true = OCR, false = native
        use crate::cli::tool_prompts::tool_prompt_line;
        use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

        let native_info = if has_native {
            format!("{} chars", native_chars)
        } else {
            "vide".to_string()
        };

        let default_label = if default_ocr { "defaut=2" } else { "defaut=1" };
        tool_prompt_line(&format!(
            "[1] Natif ({})  [2] OCR (vision)  [{}]",
            native_info, default_label
        ));

        crossterm::terminal::enable_raw_mode()?;
        let result = loop {
            if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
                if kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }
                if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                    break default_ocr;
                }
                match code {
                    KeyCode::Char('1') => break false,
                    KeyCode::Char('2') => break true,
                    KeyCode::Enter => break default_ocr,
                    KeyCode::Esc => break default_ocr,
                    _ => continue,
                }
            }
        };
        let _ = crossterm::terminal::disable_raw_mode();

        Ok(result)
    }
}
