//! Interactive REPL (Read-Eval-Print Loop)

use crate::cli::confirm::{confirm_tool_execution_with_ui, ConfirmResult};
use crate::cli::ui::{CliUI, TokenStats};
use crate::cli::ExecutionMode;
use crate::config::{Config, ToolSet};
use crate::context::{Conversation, ProjectContext};
use crate::llm::{generate_tool_prompt, generate_akari_prompt, LmStudioClient, StreamConsumer, StreamEvent, ToolParser};
use crate::tools::{ToolExecutor, ToolRegistry};
use anyhow::Result;
use colored::*;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;

/// Get the directory where the executable is located
fn get_exe_directory() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|p| p.to_string_lossy().to_string())
}

/// Estimate token count from text (roughly 4 chars = 1 token)
fn estimate_tokens(text: &str) -> usize {
    (text.len() + 3) / 4
}

/// Filtre les modeles d'embedding (text-embedding-*, *-embed-*, etc.)
fn is_embedding_model(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("embed") || n.contains("embedding") || n.contains("reranker")
}

/// Choisit le modele texte a utiliser parmi ceux loades dans LM Studio.
/// Priorite : si le modele configure est loade → on le garde. Sinon, premier non-embedding.
fn pick_loaded_text_model<'a>(models: &'a [String], configured: &str) -> Option<&'a String> {
    if !configured.is_empty() {
        if let Some(m) = models.iter().find(|m| m.as_str() == configured) {
            return Some(m);
        }
    }
    models.iter().find(|m| !is_embedding_model(m))
}

/// Resume intelligent des arguments d'un tool call pour l'affichage
fn format_tool_header(call: &crate::llm::types::ParsedToolCall) -> String {
    match call.name.as_str() {
        "bash" => call.get_string("command").unwrap_or_default(),
        "read_file" => call.get_string("path").unwrap_or_default(),
        "write_file" => call.get_string("path").unwrap_or_default(),
        "edit_file" => call.get_string("path").unwrap_or_default(),
        "glob" => call.get_string("pattern").unwrap_or_default(),
        "grep" => {
            let pattern = call.get_string("pattern").unwrap_or_default();
            let path = call.get_string("path").unwrap_or_else(|| ".".into());
            format!("{} in {}", pattern, path)
        }
        "web_fetch" => call.get_string("url").unwrap_or_default(),
        "web_search" => call.get_string("query").unwrap_or_default(),
        "list_directory" => call.get_string("path").unwrap_or_else(|| ".".into()),
        "delete" => call.get_string("path").unwrap_or_default(),
        _ => {
            let args = serde_json::to_string(&call.arguments).unwrap_or_default();
            if args.len() > 80 {
                let mut end = 80;
                while end > 0 && !args.is_char_boundary(end) { end -= 1; }
                format!("{}...", &args[..end])
            } else {
                args
            }
        }
    }
}

/// Filtre les blocs <think>...</think> du streaming GLM-4.6V
/// Retourne uniquement le texte visible pour l'utilisateur
struct ThinkFilter {
    in_think: bool,
    buffer: String,
}

impl ThinkFilter {
    fn new() -> Self {
        Self { in_think: false, buffer: String::new() }
    }

    fn reset(&mut self) {
        self.in_think = false;
        self.buffer.clear();
    }

    /// Traite un chunk de streaming, retourne la partie affichable
    fn process(&mut self, text: &str) -> String {
        self.buffer.push_str(text);
        let mut output = String::new();

        loop {
            if self.in_think {
                if let Some(end) = self.buffer.find("</think>") {
                    self.in_think = false;
                    self.buffer = self.buffer[end + 8..].to_string();
                } else {
                    // Toujours dans le bloc think, attendre la fin
                    break;
                }
            } else {
                if let Some(start) = self.buffer.find("<think>") {
                    output.push_str(&self.buffer[..start]);
                    self.in_think = true;
                    self.buffer = self.buffer[start + 7..].to_string();
                } else {
                    // Pas de <think>, verifier si le buffer se termine par un debut partiel
                    let safe = self.safe_flush_point();
                    output.push_str(&self.buffer[..safe]);
                    self.buffer = self.buffer[safe..].to_string();
                    break;
                }
            }
        }

        output
    }

    /// Flush le buffer restant
    fn flush(&mut self) -> String {
        if self.in_think {
            self.buffer.clear();
            String::new()
        } else {
            std::mem::take(&mut self.buffer)
        }
    }

    /// Trouve le point sur pour flusher (evite de couper un tag partiel)
    fn safe_flush_point(&self) -> usize {
        let buf = &self.buffer;
        // Verifier si le buffer se termine par un debut de "<think>" ou "</think>"
        for tag in &["<think>", "</think>"] {
            for i in 1..tag.len() {
                if buf.ends_with(&tag[..i]) {
                    return buf.len() - i;
                }
            }
        }
        buf.len()
    }

    /// Est-on dans un bloc think ?
    fn is_thinking(&self) -> bool {
        self.in_think
    }
}

/// Interactive REPL for Makima
pub struct Repl {
    client: LmStudioClient,
    /// Client for OCR (shared with tools)
    ocr_client: Arc<LmStudioClient>,
    executor: ToolExecutor,
    conversation: Conversation,
    project: Option<ProjectContext>,
    ui: CliUI,
    parser: ToolParser,
    /// Tools approved for the session
    approved_tools: HashSet<String>,
    /// Current execution mode (Plan or Edit)
    mode: ExecutionMode,
    /// Token statistics for display
    tokens: TokenStats,
    /// Input history for arrow key navigation
    input_history: Vec<String>,
    /// Current position in history (None = not browsing history)
    history_index: Option<usize>,
    /// Temporary storage for current input when browsing history
    temp_input: String,
    /// Current tool set (Standard or Akari)
    tool_set: ToolSet,
    /// Filtre pour les blocs <think> de GLM-4.6V
    think_filter: ThinkFilter,
}

impl Repl {
    pub async fn new(config: &Config) -> Result<Self> {
        let client = LmStudioClient::new(&config.lm_studio.url, &config.lm_studio.model)
            .with_vision_model(&config.lm_studio.vision_model)
            .with_max_tokens(config.lm_studio.max_tokens)
            .with_temperature(config.lm_studio.temperature);

        // Priorite: 1) config (defini par l'utilisateur), 2) exe dir, 3) current dir
        let working_dir = if !config.tools.working_dir.is_empty() {
            config.tools.working_dir.clone()
        } else {
            get_exe_directory()
                .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()))
                .unwrap_or_else(|| ".".to_string())
        };

        // Wrap client in Arc for sharing with tools (OCR)
        let client_arc = std::sync::Arc::new(
            LmStudioClient::new(&config.lm_studio.url, &config.lm_studio.model)
                .with_vision_model(&config.lm_studio.vision_model)
                .with_max_tokens(config.lm_studio.max_tokens)
                .with_temperature(config.lm_studio.temperature)
        );

        let registry = match config.tools.tool_set {
            ToolSet::Akari => ToolRegistry::with_akari_tools(Some(working_dir.clone()), Some(client_arc.clone())),
            ToolSet::Standard => ToolRegistry::with_defaults_and_client(Some(working_dir.clone()), Some(client_arc.clone())),
        };
        let executor = ToolExecutor::new(registry);

        let project = ProjectContext::from_directory(&working_dir).await.ok();

        // Build system prompt based on tool set
        let tool_prompt = match config.tools.tool_set {
            ToolSet::Akari => generate_akari_prompt(&executor.registry().get_definitions()),
            ToolSet::Standard => generate_tool_prompt(&executor.registry().get_definitions()),
        };
        let mut system_prompt = tool_prompt;

        if let Some(ref proj) = project {
            system_prompt.push_str("\n\n## Current Project Context\n");
            system_prompt.push_str(&proj.to_summary());
        }

        let conversation = Conversation::new().with_system_prompt(system_prompt);

        Ok(Self {
            client,
            ocr_client: client_arc,
            executor,
            conversation,
            project,
            ui: CliUI::new(),
            parser: ToolParser::new(),
            approved_tools: HashSet::new(),
            mode: ExecutionMode::default(), // Default to Edit mode
            tokens: TokenStats::default(),
            input_history: Vec::new(),
            history_index: None,
            temp_input: String::new(),
            tool_set: config.tools.tool_set,
            think_filter: ThinkFilter::new(),
        })
    }

    /// Run the REPL
    pub async fn run(&mut self) -> Result<()> {
        // Display welcome with model info
        let model_info = format!("{} Context: {} GGUF",
            self.client.model(),
            self.client.max_tokens()
        );
        self.ui.welcome_with_model(&model_info);

        // Check LM Studio connection
        if !self.client.health_check().await? {
            self.ui.error("Impossible de se connecter a LM Studio. Verifiez qu'il tourne sur l'URL configuree.");
            self.ui.info("Par defaut: http://localhost:1234/v1");
            return Ok(());
        }

        self.ui.success("Connecte a LM Studio");

        // Auto-detect : si le modele configure n'est pas loade, prendre le premier loade
        match self.client.list_models().await {
            Ok(models) if !models.is_empty() => {
                let configured = self.client.model().to_string();
                if let Some(picked) = pick_loaded_text_model(&models, &configured) {
                    if picked != &configured {
                        self.client.set_model(picked);
                        self.ui.info(&format!("Auto-detect : modele texte = {}", picked));
                    }
                }
            }
            _ => {}
        }

        if let Some(ref project) = self.project {
            self.ui.info(&format!("Project: {} ({})", project.name, project.project_type.map(|t| t.as_str()).unwrap_or("Unknown")));
        }

        println!();

        // Initialize the fixed panel at the bottom
        self.ui.init_fixed_panel(self.mode);

        loop {
            let input = match self.read_input() {
                Ok(Some(input)) => input,
                Ok(None) => continue,
                Err(_) => break,
            };

            let input = input.trim();
            if input.is_empty() {
                // Redraw fixed panel on empty input
                self.ui.redraw_fixed_panel_full(self.mode, "", false, &self.tokens);
                continue;
            }

            // Handle commands
            if input.starts_with('/') {
                match self.handle_command(input).await {
                    Ok(true) => break, // Exit command
                    Ok(false) => {
                        self.ui.redraw_fixed_panel_full(self.mode, "", false, &self.tokens);
                        continue;
                    }
                    Err(e) => {
                        self.ui.error(&format!("Command error: {}", e));
                        self.ui.redraw_fixed_panel_full(self.mode, "", false, &self.tokens);
                        continue;
                    }
                }
            }

            // Process user message
            if let Err(e) = self.process_message(input).await {
                self.ui.error(&format!("Error: {}", e));
            }

            println!();
            // Ensure scroll region and redraw fixed panel
            self.ui.ensure_scroll_region();
            self.ui.redraw_fixed_panel_full(self.mode, "", false, &self.tokens);
        }

        self.ui.cleanup_fixed_panel();
        self.ui.info("Au revoir !");
        Ok(())
    }

    /// Toggle between Plan and Edit modes
    fn toggle_mode(&mut self) {
        self.mode = self.mode.toggle();
        // Just update the status line - no println to avoid duplication
        self.ui.update_status_line(self.mode);
    }

    /// Read user input with line editing, history navigation, and Shift+Tab detection
    fn read_input(&mut self) -> io::Result<Option<String>> {
        let mut input = String::new();

        // Reset history navigation state
        self.history_index = None;
        self.temp_input.clear();

        // Update prompt line to show empty input
        self.ui.update_prompt_line("");
        self.ui.move_to_prompt_input(0);

        // Enable raw mode for key detection
        if enable_raw_mode().is_err() {
            // Fallback to simple readline if raw mode fails
            let _ = disable_raw_mode();
            print!("> ");
            io::stdout().flush()?;
            io::stdin().read_line(&mut input)?;
            // Save to history if not empty
            let trimmed = input.trim().to_string();
            if !trimmed.is_empty() {
                self.input_history.push(trimmed);
            }
            return Ok(Some(input));
        }

        loop {
            if event::poll(std::time::Duration::from_millis(100))? {
                let evt = event::read()?;
                if let Event::Resize(_, _) = evt {
                    // Terminal resized: redraw the fixed panel
                    self.ui.redraw_fixed_panel();
                    self.ui.move_to_prompt_input(input.len());
                    continue;
                }
                if let Event::Key(key_event) = evt {
                    // Only process key press events (not release)
                    if key_event.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }

                    match (key_event.modifiers, key_event.code) {
                        // Shift+Tab = toggle mode
                        (modifiers, KeyCode::BackTab) if modifiers.contains(KeyModifiers::SHIFT) => {
                            self.toggle_mode();
                            // Keep cursor at prompt position
                            self.ui.move_to_prompt_input(input.len());
                        }
                        // BackTab without shift also toggles (some terminals send this)
                        (_, KeyCode::BackTab) => {
                            self.toggle_mode();
                            self.ui.move_to_prompt_input(input.len());
                        }
                        // Up arrow = previous history
                        (_, KeyCode::Up) => {
                            if !self.input_history.is_empty() {
                                match self.history_index {
                                    None => {
                                        // Start browsing history, save current input
                                        self.temp_input = input.clone();
                                        self.history_index = Some(self.input_history.len() - 1);
                                        input = self.input_history[self.input_history.len() - 1].clone();
                                    }
                                    Some(idx) if idx > 0 => {
                                        // Go further back in history
                                        self.history_index = Some(idx - 1);
                                        input = self.input_history[idx - 1].clone();
                                    }
                                    _ => {
                                        // Already at oldest entry, do nothing
                                    }
                                }
                                self.ui.update_prompt_line(&input);
                                self.ui.move_to_prompt_input(input.len());
                            }
                        }
                        // Down arrow = next history
                        (_, KeyCode::Down) => {
                            if let Some(idx) = self.history_index {
                                if idx + 1 < self.input_history.len() {
                                    // Go forward in history
                                    self.history_index = Some(idx + 1);
                                    input = self.input_history[idx + 1].clone();
                                } else {
                                    // Return to current input
                                    self.history_index = None;
                                    input = self.temp_input.clone();
                                }
                                self.ui.update_prompt_line(&input);
                                self.ui.move_to_prompt_input(input.len());
                            }
                        }
                        // Enter = submit
                        (_, KeyCode::Enter) => {
                            let _ = disable_raw_mode();
                            // Save to history if not empty and different from last entry
                            let trimmed = input.trim().to_string();
                            if !trimmed.is_empty() {
                                let should_add = self.input_history.last()
                                    .map(|last| last != &trimmed)
                                    .unwrap_or(true);
                                if should_add {
                                    self.input_history.push(trimmed);
                                }
                            }
                            // Move to scrollable area for output
                            self.ui.ensure_scroll_region();
                            println!();
                            break;
                        }
                        // Ctrl+C = cancel
                        (modifiers, KeyCode::Char('c')) if modifiers.contains(KeyModifiers::CONTROL) => {
                            let _ = disable_raw_mode();
                            input.clear();
                            self.ui.update_prompt_line("");
                            return Ok(None);
                        }
                        // Ctrl+D = exit
                        (modifiers, KeyCode::Char('d')) if modifiers.contains(KeyModifiers::CONTROL) => {
                            let _ = disable_raw_mode();
                            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"));
                        }
                        // Regular character - reset history browsing
                        (_, KeyCode::Char(c)) => {
                            // If typing while browsing history, stay on current text
                            if self.history_index.is_some() {
                                self.history_index = None;
                            }
                            input.push(c);
                            self.ui.update_prompt_line(&input);
                            self.ui.move_to_prompt_input(input.len());
                        }
                        // Backspace
                        (_, KeyCode::Backspace) => {
                            if !input.is_empty() {
                                if self.history_index.is_some() {
                                    self.history_index = None;
                                }
                                input.pop();
                                self.ui.update_prompt_line(&input);
                                self.ui.move_to_prompt_input(input.len());
                            }
                        }
                        // Escape = clear line
                        (_, KeyCode::Esc) => {
                            self.history_index = None;
                            input.clear();
                            self.ui.update_prompt_line("");
                            self.ui.move_to_prompt_input(0);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(Some(input))
    }

    /// Handle slash commands
    async fn handle_command(&mut self, command: &str) -> Result<bool> {
        let parts: Vec<&str> = command.splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();
        let args = parts.get(1).copied();

        match cmd.as_str() {
            "/exit" | "/quit" | "/q" | "/quitter" => {
                return Ok(true);
            }
            "/help" | "/h" | "/?" | "/aide" => {
                self.ui.help();
                // Redraw the fixed panel after help closes
                self.ui.init_fixed_panel(self.mode);
            }
            "/clear" | "/effacer" => {
                self.conversation.clear();
                self.ui.success("Conversation effacee");
            }
            "/new" | "/nouveau" => {
                // Build new system prompt based on current tool set
                let tool_prompt = match self.tool_set {
                    ToolSet::Akari => generate_akari_prompt(&self.executor.registry().get_definitions()),
                    ToolSet::Standard => generate_tool_prompt(&self.executor.registry().get_definitions()),
                };
                let mut system_prompt = tool_prompt;

                if let Some(ref proj) = self.project {
                    system_prompt.push_str("\n\n## Contexte du Projet\n");
                    system_prompt.push_str(&proj.to_summary());
                }

                self.conversation = Conversation::new().with_system_prompt(system_prompt);
                self.ui.success("Nouvelle conversation demarree");
            }
            "/config" | "/configuration" => {
                self.ui.header("Configuration Actuelle");
                if let Some(ref proj) = self.project {
                    println!("Project: {} at {}", proj.name, proj.root.display());
                }
                println!("Conversation: {} messages", self.conversation.message_count());
                println!("Outils approuves: {:?}", self.approved_tools);
                println!("Max tokens: {}", self.client.max_tokens());
                println!("Mode: {}", self.mode.display_name());
            }
            "/tools" | "/outils" => {
                self.ui.header("Outils Disponibles");
                for name in self.executor.registry().names() {
                    if let Some(tool) = self.executor.registry().get(&name) {
                        let approved = if self.approved_tools.contains(&name) {
                            " ✓".green().to_string()
                        } else {
                            String::new()
                        };
                        println!("  {} - {}{}", name.cyan(), tool.description(), approved);
                    }
                }
            }
            "/history" | "/historique" => {
                self.ui.header("Historique de Conversation");
                for (i, msg) in self.conversation.messages.iter().enumerate() {
                    let role = format!("{:?}", msg.role).to_uppercase();
                    let preview = msg.content.as_ref()
                        .map(|c| if c.len() > 80 { format!("{}...", &c[..80]) } else { c.clone() })
                        .unwrap_or_else(|| "[tool call]".to_string());
                    println!("  {}. {} {}", i + 1, role.cyan(), preview.dimmed());
                }
            }
            "/plan" => {
                self.mode = ExecutionMode::Plan;
                self.ui.mode_changed(self.mode);
                self.ui.update_status_line(self.mode);
            }
            "/edit" => {
                self.mode = ExecutionMode::Edit;
                self.ui.mode_changed(self.mode);
                self.ui.update_status_line(self.mode);
            }
            "/mode" => {
                self.ui.header("Mode Actuel");
                self.ui.status_bar(self.mode);
            }
            "/modeles" | "/models" => {
                self.ui.header("Modeles Disponibles (LM Studio)");
                match self.client.list_models().await {
                    Ok(models) => {
                        let current = self.client.model();
                        let vision = self.client.vision_model();
                        for m in &models {
                            let tag_text = if m == current && m == vision {
                                " (texte + vision)".green().to_string()
                            } else if m == current {
                                " (texte)".green().to_string()
                            } else if m == vision {
                                " (vision)".cyan().to_string()
                            } else {
                                String::new()
                            };
                            println!("  {}{}", m, tag_text);
                        }
                        println!();
                        self.ui.info("Utilisez /modele <nom> pour changer le modele texte");
                    }
                    Err(e) => {
                        self.ui.error(&format!("Impossible de recuperer la liste: {}", e));
                    }
                }
            }
            "/auto" => {
                match self.client.list_models().await {
                    Ok(models) if !models.is_empty() => {
                        let current = self.client.model().to_string();
                        match models.iter().find(|m| !is_embedding_model(m)) {
                            Some(picked) if picked != &current => {
                                self.client.set_model(picked);
                                self.ui.success(&format!("Modele texte bascule sur : {}", picked));
                            }
                            Some(_) => {
                                self.ui.info(&format!("Deja sur le bon modele : {}", current));
                            }
                            None => {
                                self.ui.warning("Aucun modele texte loade dans LM Studio (que des embeddings)");
                            }
                        }
                    }
                    Ok(_) => self.ui.warning("Aucun modele loade dans LM Studio"),
                    Err(e) => self.ui.error(&format!("Impossible de recuperer la liste: {}", e)),
                }
            }
            "/modele" | "/model" => {
                match args {
                    None | Some("") => {
                        self.ui.header("Modeles actuels");
                        println!("  Texte : {}", self.client.model().green());
                        println!("  Vision: {}", self.client.vision_model().cyan());
                        println!();
                        self.ui.info("Usage: /modele <nom>  (tapez /modeles pour la liste)");
                    }
                    Some(name) => {
                        let name = name.trim();
                        // Verifier que le modele existe dans LM Studio
                        match self.client.list_models().await {
                            Ok(models) => {
                                if models.iter().any(|m| m == name) {
                                    self.client.set_model(name);
                                    self.ui.success(&format!("Modele texte change : {}", name));
                                } else {
                                    self.ui.warning(&format!("Modele '{}' non trouve dans LM Studio", name));
                                    self.ui.info("Tapez /modeles pour voir les modeles disponibles");
                                }
                            }
                            Err(e) => {
                                self.ui.error(&format!("Impossible de verifier le modele: {}", e));
                            }
                        }
                    }
                }
            }
            "/espace" | "/workdir" => {
                let current_dir = std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| ".".to_string());

                self.ui.header("Repertoire de Travail");
                println!("  Actuel: {}", self.executor.registry().working_dir().green());
                println!();
                println!("  {} Garder l'actuel", "1.".dimmed());
                println!("  {} Repertoire courant: {}", "2.".dimmed(), current_dir.cyan());
                println!("  {} Personnalise", "3.".dimmed());
                print!("  {} Choix [1/2/3]: ", "▶".green());
                io::stdout().flush()?;

                let mut choice = String::new();
                io::stdin().read_line(&mut choice)?;

                let new_workdir = match choice.trim() {
                    "2" => Some(current_dir),
                    "3" => {
                        print!("  {} Chemin: ", "▶".green());
                        io::stdout().flush()?;
                        let mut path = String::new();
                        io::stdin().read_line(&mut path)?;
                        let path = path.trim().to_string();
                        if !path.is_empty() && std::path::Path::new(&path).exists() {
                            Some(path)
                        } else {
                            self.ui.warning("Chemin invalide, repertoire non modifie");
                            None
                        }
                    }
                    _ => None,
                };

                if let Some(workdir) = new_workdir {
                    // Recreate registry with new working dir and preserved OCR client
                    let registry = match self.tool_set {
                        ToolSet::Akari => crate::tools::ToolRegistry::with_akari_tools(
                            Some(workdir.clone()),
                            Some(self.ocr_client.clone()),
                        ),
                        ToolSet::Standard => crate::tools::ToolRegistry::with_defaults_and_client(
                            Some(workdir.clone()),
                            Some(self.ocr_client.clone()),
                        ),
                    };
                    self.executor = crate::tools::ToolExecutor::new(registry);
                    self.ui.success(&format!("Repertoire de travail: {}", workdir));
                }
            }
            _ => {
                self.ui.warning(&format!("Commande inconnue: {}", cmd));
                self.ui.info("Tapez /help pour les commandes disponibles");
            }
        }

        Ok(false)
    }

    /// Process a user message
    async fn process_message(&mut self, input: &str) -> Result<()> {
        // Display user prompt with timestamp
        self.ui.user_prompt(input);
        println!();

        self.conversation.add_user_message(input);

        // Estimate tokens for the input
        self.tokens.sent = estimate_tokens(input);
        self.tokens.received = 0;

        // Run the conversation loop (may involve multiple tool calls)
        loop {
            let messages = self.conversation.get_messages();
            let tools = Some(self.executor.registry().get_definitions());

            // Show working indicator
            self.ui.update_work_line(true, "Reflexion...", &self.tokens);

            // Stream the response
            let mut rx = self.client.chat_stream(messages, tools).await?;
            let mut consumer = StreamConsumer::new();

            // Print streaming content
            print!("{} ", "🤖".cyan());
            io::stdout().flush()?;
            self.think_filter.reset();
            let mut shown_thinking = false;
            let stream_start = std::time::Instant::now();

            while let Some(event) = rx.recv().await {
                match &event {
                    StreamEvent::Content(text) => {
                        // Update received tokens
                        self.tokens.received += estimate_tokens(text);

                        // Filtrer les blocs <think>...</think>
                        let visible = self.think_filter.process(text);

                        let elapsed = stream_start.elapsed().as_secs_f64();

                        if self.think_filter.is_thinking() {
                            // Afficher "Reflexion..." avec duree pendant le raisonnement
                            if !shown_thinking {
                                shown_thinking = true;
                            }
                            self.ui.update_work_line_with_time(true, "Reflexion...", &self.tokens, elapsed);
                        } else {
                            if shown_thinking {
                                shown_thinking = false;
                            }
                            self.ui.update_work_line_with_time(true, "Generation...", &self.tokens, elapsed);
                        }

                        // Afficher le texte visible (sans tool tags ni think)
                        if !visible.is_empty() && !self.parser.contains_tool_calls(&visible) {
                            self.ui.stream_content(&visible);
                        }
                    }
                    StreamEvent::ToolCallStart { .. } => {
                        // L'affichage se fera au moment de l'execution (avec les args)
                        println!();
                    }
                    StreamEvent::Error(e) => {
                        self.ui.error(e);
                    }
                    _ => {}
                }
                consumer.accumulator.process_event(event);

                if consumer.accumulator.done {
                    break;
                }
            }

            // Flush du filtre think (texte restant apres fin du stream)
            let remaining = self.think_filter.flush();
            if !remaining.is_empty() && !self.parser.contains_tool_calls(&remaining) {
                self.ui.stream_content(&remaining);
            }

            // Finalize tokens for this request
            self.tokens.finalize_request();
            self.ui.update_work_line(false, "", &self.tokens);

            println!();

            // Get any tool calls
            let tool_calls = consumer.get_tool_calls();
            let clean_content = consumer.get_clean_content();

            // Synchroniser stdout/stderr avant exécution des tools
            if !tool_calls.is_empty() {
                let _ = io::stdout().flush();
                let _ = io::stderr().flush();
                // Petit délai pour laisser le terminal traiter les buffers
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            // Add assistant message to history
            if !clean_content.is_empty() || tool_calls.is_empty() {
                self.conversation.add_assistant_message(&clean_content);
            }

            // If no tool calls, we're done
            if tool_calls.is_empty() {
                break;
            }

            // Handle tool calls based on execution mode
            let mut should_continue = true;

            for call in &tool_calls {
                // In Plan mode, show tool preview but don't execute
                if self.mode == ExecutionMode::Plan {
                    let args_str = serde_json::to_string(&call.arguments).unwrap_or_default();
                    self.ui.plan_tool_preview(&call.name, &args_str);
                    // Add a fake result so the conversation can continue
                    self.conversation.add_tool_result(
                        &call.id,
                        "[Mode Plan] Outil non execute - passez en mode Edit pour executer"
                    );
                    continue;
                }

                // Edit mode: normal execution with confirmation
                if let Some(tool) = self.executor.registry().get(&call.name) {
                    if tool.requires_confirmation() && !self.approved_tools.contains(&call.name) {
                        let summary = tool.summarize_call(call);
                        match confirm_tool_execution_with_ui(&self.ui, &call.name, &summary)? {
                            ConfirmResult::Yes => {
                                // Execute just this once
                            }
                            ConfirmResult::Always => {
                                self.approved_tools.insert(call.name.clone());
                            }
                            ConfirmResult::No => {
                                self.conversation.add_tool_result(&call.id, "L'utilisateur a refuse l'execution de cet outil");
                                continue;
                            }
                            ConfirmResult::Cancel => {
                                self.conversation.add_tool_result(&call.id, "L'utilisateur a annule l'operation");
                                should_continue = false;
                                break;
                            }
                        }
                    }

                    // Afficher le header Claude-style avant execution
                    let header = format_tool_header(call);
                    self.ui.tool_execution_start(&call.name, &header);

                    // Execute the tool avec timer
                    let start = std::time::Instant::now();
                    match tool.execute(call).await {
                        Ok(result) => {
                            let duration = start.elapsed().as_secs_f64();
                            self.ui.tool_execution_end(result.success, &result.content, duration);
                            self.conversation.add_tool_result(&call.id, &result.content);
                        }
                        Err(e) => {
                            let duration = start.elapsed().as_secs_f64();
                            let error_msg = format!("Error: {}", e);
                            self.ui.tool_execution_end(false, &error_msg, duration);
                            self.conversation.add_tool_result(&call.id, &error_msg);
                        }
                    }
                } else {
                    let error_msg = format!("Unknown tool: {}", call.name);
                    self.ui.error(&error_msg);
                    self.conversation.add_tool_result(&call.id, &error_msg);
                }
            }

            if !should_continue {
                break;
            }

            // In Plan mode, stop after showing tool previews (don't continue the loop)
            if self.mode == ExecutionMode::Plan {
                self.ui.info("Mode Plan: outils affiches mais non executes. Utilisez /edit pour passer en mode execution.");
                break;
            }

            // Continue the loop to let the model respond to tool results
        }

        Ok(())
    }
}
