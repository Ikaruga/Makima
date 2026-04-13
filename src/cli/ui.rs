//! CLI UI formatting and display

use crate::cli::ExecutionMode;
use colored::*;
use crossterm::{
    cursor::{MoveTo, SavePosition, RestorePosition},
    execute,
    terminal::{self, Clear, ClearType},
};
use std::io::{self, Write};

/// Token statistics for display
#[derive(Default, Clone)]
pub struct TokenStats {
    /// Tokens sent in current request
    pub sent: usize,
    /// Tokens received in current request
    pub received: usize,
    /// Total tokens for the session
    pub session_total: usize,
}

impl TokenStats {
    /// Update session total after a request
    pub fn finalize_request(&mut self) {
        self.session_total += self.sent + self.received;
        self.sent = 0;
        self.received = 0;
    }

    /// Format tokens for display (e.g., "1.2k")
    fn format_count(count: usize) -> String {
        if count >= 1000 {
            format!("{:.1}k", count as f64 / 1000.0)
        } else {
            count.to_string()
        }
    }
}

/// Number of lines reserved for the fixed panel at the bottom
/// Layout (8 lines):
///   Line 1: Tool zone line 1
///   Line 2: Tool zone line 2
///   Line 3: Separator (tool zone bottom)
///   Line 4: Work line (spinner + tokens)
///   Line 5: Separator
///   Line 6: Prompt > input
///   Line 7: Separator
///   Line 8: Status (mode + shortcuts)
const FIXED_PANEL_LINES: u16 = 8;
const _TOOL_ZONE_LINES: u16 = 3; // 2 lines + separator

/// Set scroll region using raw ANSI escape sequence
/// top and bottom are 1-indexed row numbers
fn set_scroll_region(top: u16, bottom: u16) {
    // ANSI: ESC [ top ; bottom r
    print!("\x1b[{};{}r", top, bottom);
    let _ = io::stdout().flush();
}

/// Reset scroll region to full screen
fn reset_scroll_region() {
    // ANSI: ESC [ r (reset to default)
    print!("\x1b[r");
    let _ = io::stdout().flush();
}

/// CLI UI helper
pub struct CliUI {
    /// Whether to use colors
    use_colors: bool,
    /// Width for wrapping (0 = no wrap)
    width: usize,
}

impl Default for CliUI {
    fn default() -> Self {
        Self::new()
    }
}

impl CliUI {
    pub fn new() -> Self {
        let width = terminal_size::terminal_size()
            .map(|(w, _)| w.0 as usize)
            .unwrap_or(80);

        Self {
            use_colors: true,
            width,
        }
    }

    /// Print a header
    pub fn header(&self, text: &str) {
        println!();
        if self.use_colors {
            println!("{}", text.bold().cyan());
            println!("{}", "─".repeat(text.len().min(self.width)).dimmed());
        } else {
            println!("{}", text);
            println!("{}", "─".repeat(text.len().min(self.width)));
        }
    }

    /// Print an info message
    pub fn info(&self, text: &str) {
        if self.use_colors {
            println!("{} {}", "ℹ".blue(), text);
        } else {
            println!("[INFO] {}", text);
        }
    }

    /// Print a success message
    pub fn success(&self, text: &str) {
        if self.use_colors {
            println!("{} {}", "✓".green(), text.green());
        } else {
            println!("[OK] {}", text);
        }
    }

    /// Print an error message
    pub fn error(&self, text: &str) {
        if self.use_colors {
            eprintln!("{} {}", "✗".red(), text.red());
        } else {
            eprintln!("[ERROR] {}", text);
        }
    }

    /// Print a warning message
    pub fn warning(&self, text: &str) {
        if self.use_colors {
            println!("{} {}", "⚠".yellow(), text.yellow());
        } else {
            println!("[WARN] {}", text);
        }
    }

    /// Print assistant response
    pub fn assistant(&self, text: &str) {
        if self.use_colors {
            println!("{} {}", "🤖".to_string().cyan(), text);
        } else {
            println!("[MAKIMA] {}", text);
        }
    }

    /// Print user prompt with timestamp
    pub fn user_prompt(&self, text: &str) {
        let now = chrono::Local::now();
        let timestamp = now.format("%H:%M:%S").to_string();

        if self.use_colors {
            println!("{} {} {}", timestamp.dimmed(), "👤".to_string(), text);
        } else {
            println!("[{}] USER: {}", timestamp, text);
        }
    }

    /// Print a tool call
    pub fn tool_call(&self, name: &str, summary: &str) {
        if self.use_colors {
            println!("{} {} {}", "🔧".to_string().yellow(), name.cyan().bold(), summary.dimmed());
        } else {
            println!("[TOOL] {} - {}", name, summary);
        }
    }

    /// Print tool result
    pub fn tool_result(&self, success: bool, content: &str) {
        let prefix = if success {
            if self.use_colors {
                "📤".to_string().green().to_string()
            } else {
                "[RESULT]".to_string()
            }
        } else {
            if self.use_colors {
                "❌".to_string().red().to_string()
            } else {
                "[ERROR]".to_string()
            }
        };

        // Limit output length
        let max_len = 2000;
        let display = if content.len() > max_len {
            let mut end = max_len;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}... (tronque, {} caracteres au total)", &content[..end], content.len())
        } else {
            content.to_string()
        };

        println!("{}", prefix);
        for line in display.lines() {
            println!("  {}", line.dimmed());
        }
    }

    /// Print tool execution header (Claude Code style)
    /// Ex: `Bash(cargo build --release)`
    pub fn tool_execution_start(&self, name: &str, args_summary: &str) {
        let display_name = match name {
            "bash" => "Bash",
            "read_file" => "Read",
            "write_file" => "Write",
            "edit_file" => "Edit",
            "glob" => "Glob",
            "grep" => "Grep",
            "list_directory" => "ListDir",
            "delete" => "Delete",
            "web_fetch" => "WebFetch",
            "web_search" => "WebSearch",
            "csv_to_docx" => "CsvToDocx",
            "pdf_to_txt" => "PdfToTxt",
            "format_liasse_fiscale" => "FormatLiasse",
            other => other,
        };

        // Tronquer les args si trop longs
        let max_args = 100;
        let args_display = if args_summary.len() > max_args {
            let mut end = max_args;
            while end > 0 && !args_summary.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &args_summary[..end])
        } else {
            args_summary.to_string()
        };

        if self.use_colors {
            println!("{}({})", display_name.cyan().bold(), args_display.dimmed());
        } else {
            println!("{}({})", display_name, args_display);
        }
    }

    /// Print tool execution result (Claude Code style)
    /// Ex:
    ///   ⎿  Compiling makima v0.1.1
    ///      Finished release in 52s
    ///      (2.3s)
    pub fn tool_execution_end(&self, success: bool, content: &str, duration_secs: f64) {
        let lines: Vec<&str> = content.lines().collect();
        let max_lines = 30;
        let display_lines = if lines.len() > max_lines {
            &lines[..max_lines]
        } else {
            &lines[..]
        };

        for (i, line) in display_lines.iter().enumerate() {
            if i == 0 {
                if success {
                    if self.use_colors {
                        println!("  {} {}", "⎿ ".dimmed(), line.dimmed());
                    } else {
                        println!("  ⎿  {}", line);
                    }
                } else {
                    if self.use_colors {
                        println!("  {} {}", "⎿ ".red(), line.red());
                    } else {
                        println!("  ⎿  ! {}", line);
                    }
                }
            } else {
                if self.use_colors {
                    if success {
                        println!("     {}", line.dimmed());
                    } else {
                        println!("     {}", line.red());
                    }
                } else {
                    println!("     {}", line);
                }
            }
        }

        // Lignes tronquees
        if lines.len() > max_lines {
            let remaining = lines.len() - max_lines;
            if self.use_colors {
                println!("     {}", format!("... +{} lignes", remaining).dimmed());
            } else {
                println!("     ... +{} lignes", remaining);
            }
        }

        // Duree
        let duration_str = if duration_secs >= 1.0 {
            format!("({:.1}s)", duration_secs)
        } else {
            format!("({:.0}ms)", duration_secs * 1000.0)
        };

        if self.use_colors {
            println!("     {}", duration_str.dimmed());
        } else {
            println!("     {}", duration_str);
        }
    }

    /// Print a code block
    pub fn code_block(&self, code: &str, language: Option<&str>) {
        if self.use_colors {
            println!("{}", format!("```{}", language.unwrap_or("")).dimmed());
            for line in code.lines() {
                println!("  {}", line);
            }
            println!("{}", "```".dimmed());
        } else {
            println!("---");
            for line in code.lines() {
                println!("  {}", line);
            }
            println!("---");
        }
    }

    /// Print a horizontal rule
    pub fn rule(&self) {
        if self.use_colors {
            println!("{}", "─".repeat(self.width.min(60)).dimmed());
        } else {
            println!("{}", "-".repeat(self.width.min(60)));
        }
    }

    /// Print streaming content (no newline)
    pub fn stream_content(&self, text: &str) {
        print!("{}", text);
        let _ = io::stdout().flush();
    }

    /// Print the input zone separator (full width, matches status bar)
    pub fn input_separator(&self) {
        let (cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
        if self.use_colors {
            println!("{}", "─".repeat(cols as usize).dimmed());
        } else {
            println!("{}", "-".repeat(cols as usize));
        }
    }

    /// Print the prompt for user input
    pub fn prompt(&self) {
        if self.use_colors {
            print!("{} ", ">".green().bold());
        } else {
            print!("> ");
        }
        let _ = io::stdout().flush();
    }

    /// Clear the current line
    pub fn clear_line(&self) {
        print!("\r{}\r", " ".repeat(self.width));
        let _ = io::stdout().flush();
    }

    /// Show a spinner with message
    pub fn spinner_frame(&self, frame: usize, message: &str) {
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = frames[frame % frames.len()];

        if self.use_colors {
            print!("\r{} {}", spinner.cyan(), message.dimmed());
        } else {
            print!("\r{} {}", spinner, message);
        }
        let _ = io::stdout().flush();
    }

    /// Print welcome message
    pub fn welcome(&self) {
        self.welcome_with_model("LM Studio (Local)");
    }

    /// Print welcome message with model info
    pub fn welcome_with_model(&self, model_name: &str) {
        use crossterm::{execute, terminal::{Clear, ClearType}};

        let version = env!("CARGO_PKG_VERSION");
        let model_display = model_name;

        // Clear screen and move cursor to top
        let _ = execute!(io::stdout(), Clear(ClearType::All));
        print!("\x1B[H"); // Move cursor to top-left
        let _ = io::stdout().flush();

        // Banner with ASCII art + model box
        let version_line = format!("║{:^118}║", format!("v{}", version));
        let model_line = format!("║   {:<115}║", model_display);

        let lines = [
            "╔══════════════════════════════════════════════════════════════════════════════════════════════════════════════════════╗",
            &version_line,
            "║ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ║",
            "║ ░                                     __  __    _    _  _____ __  __    _                                          ░ ║",
            "║ ░                                    |  \\/  |  / \\  | |/ /_ _|  \\/  |  / \\                                         ░ ║",
            "║ ░                                    | |\\/| | / _ \\ | ' / | || |\\/| | / _ \\                                        ░ ║",
            "║ ░                                    | |  | |/ ___ \\| . \\ | || |  | |/ ___ \\                                       ░ ║",
            "║ ░                                    |_|  |_/_/   \\_\\_|\\_\\___|_|  |_/_/   \\_\\ 牧間                                 ░ ║",
            "║ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ║",
            &model_line,
            "╚══════════════════════════════════════════════════════════════════════════════════════════════════════════════════════╝",
        ];

        if self.use_colors {
            for line in &lines {
                println!("{}", line.cyan());
            }
            println!();
            println!("{}", "   > /aide pour les commandes".bright_black());
            println!("{}", "   > Serveur web sur http://localhost:3000".bright_black());
        } else {
            for line in &lines {
                println!("{}", line);
            }
            println!();
            println!("   > /aide pour les commandes");
            println!("   > Serveur web sur http://localhost:3000");
        }
        println!();
    }

    /// Print help message (interactive selector with up/down arrows)
    pub fn help(&self) {
        use crossterm::event::{self, Event, KeyCode, KeyEvent};
        use crossterm::cursor;
        use crossterm::execute;

        let commands = [
            ("/aide", "Afficher ce message d'aide"),
            ("/effacer", "Effacer l'historique de conversation"),
            ("/nouveau", "Demarrer une nouvelle conversation"),
            ("/espace", "Changer l'espace de travail"),
            ("/outils", "Lister les outils disponibles"),
            ("/modeles", "Lister les modeles disponibles dans LM Studio"),
            ("/modele <nom>", "Changer le modele texte (vision auto si besoin d'image)"),
            ("/plan", "Passer en mode Plan (exploration seulement)"),
            ("/edit", "Passer en mode Edit (execution autorisee)"),
            ("/configuration", "Afficher la configuration actuelle"),
            ("/historique", "Afficher l'historique de conversation"),
            ("/quitter", "Quitter Makima"),
        ];

        let shortcuts = [
            ("Maj+Tab", "Basculer entre mode Plan et Edit"),
            ("Fleches", "Naviguer dans le menu"),
            ("Entree", "Selectionner / Fermer"),
            ("Echap", "Fermer le menu"),
        ];

        let mut selected: usize = 0;
        let total_items = commands.len();

        // Clear screen and hide bottom panel
        let _ = execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0));

        // Draw function
        let draw = |sel: usize, use_colors: bool| {
            // Move to top
            print!("\x1B[H");

            // Header
            if use_colors {
                println!("\n{}", "  Commandes Disponibles".bold().cyan());
                println!("{}", "  ─────────────────────".dimmed());
            } else {
                println!("\n  Commandes Disponibles");
                println!("  ─────────────────────");
            }
            println!();

            // Commands with selector
            for (i, (cmd, desc)) in commands.iter().enumerate() {
                if i == sel {
                    if use_colors {
                        println!("  {} {} {}", ">".cyan().bold(), cmd.cyan().bold(), desc.white());
                    } else {
                        println!("  > {} {}", cmd, desc);
                    }
                } else {
                    if use_colors {
                        println!("    {} {}", cmd.white(), desc.dimmed());
                    } else {
                        println!("    {} {}", cmd, desc);
                    }
                }
            }

            println!();

            // Shortcuts header
            if use_colors {
                println!("{}", "  Raccourcis Clavier".bold().cyan());
                println!("{}", "  ──────────────────".dimmed());
            } else {
                println!("  Raccourcis Clavier");
                println!("  ──────────────────");
            }
            println!();

            for (key, desc) in shortcuts.iter() {
                if use_colors {
                    println!("    {} {}", key.cyan(), desc.dimmed());
                } else {
                    println!("    {} {}", key, desc);
                }
            }

            println!();
            if use_colors {
                println!("{}", "  Fleches pour naviguer, Entree pour selectionner, Echap pour fermer".dimmed());
            } else {
                println!("  Fleches pour naviguer, Entree pour selectionner, Echap pour fermer");
            }

            let _ = io::stdout().flush();
        };

        // Initial draw
        draw(selected, self.use_colors);

        // Interactive loop
        if terminal::enable_raw_mode().is_ok() {
            loop {
                if let Ok(Event::Key(KeyEvent { code, kind, .. })) = event::read() {
                    if kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    match code {
                        KeyCode::Up => {
                            selected = if selected == 0 { total_items - 1 } else { selected - 1 };
                            draw(selected, self.use_colors);
                        }
                        KeyCode::Down => {
                            selected = (selected + 1) % total_items;
                            draw(selected, self.use_colors);
                        }
                        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                            break;
                        }
                        _ => continue,
                    }
                }
            }
            let _ = terminal::disable_raw_mode();
        }

        // Clear help screen
        let _ = execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0));
        let _ = io::stdout().flush();
    }

    /// Print the status bar showing current mode (inline version)
    pub fn status_bar(&self, mode: ExecutionMode) {
        let mode_str = match mode {
            ExecutionMode::Plan => {
                if self.use_colors {
                    mode.display_name().cyan().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
            ExecutionMode::Edit => {
                if self.use_colors {
                    mode.display_name().green().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
        };

        let shortcuts = if self.use_colors {
            "Maj+Tab: changer mode | /aide".dimmed().to_string()
        } else {
            "Maj+Tab: changer mode | /aide".to_string()
        };

        println!("  {}                    {}", mode_str, shortcuts);
    }

    /// Draw the status bar fixed at the bottom of the terminal
    pub fn draw_fixed_status_bar(&self, mode: ExecutionMode) {
        let mut stdout = io::stdout();

        // Get terminal size
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        // Build the status bar content
        let mode_str = match mode {
            ExecutionMode::Plan => {
                if self.use_colors {
                    mode.display_name().cyan().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
            ExecutionMode::Edit => {
                if self.use_colors {
                    mode.display_name().green().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
        };

        let shortcuts = if self.use_colors {
            "Maj+Tab: mode | /aide".dimmed().to_string()
        } else {
            "Maj+Tab: mode | /aide".to_string()
        };

        // Create separator line
        let separator = if self.use_colors {
            "─".repeat(cols as usize).dimmed().to_string()
        } else {
            "-".repeat(cols as usize)
        };

        // Save cursor position, move to bottom, draw, restore
        let _ = execute!(stdout, SavePosition);

        // Move to second-to-last row for separator
        let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(2)));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        print!("{}", separator);

        // Move to last row for status
        let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(1)));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        print!("  {}    {}", mode_str, shortcuts);

        // Restore cursor position
        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Clear the fixed status bar (when exiting)
    pub fn clear_fixed_status_bar(&self) {
        let mut stdout = io::stdout();
        let (_, rows) = terminal::size().unwrap_or((80, 24));

        let _ = execute!(stdout, SavePosition);
        let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(2)));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(1)));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Get the usable terminal height (excluding status bar)
    pub fn usable_height(&self) -> u16 {
        let (_, rows) = terminal::size().unwrap_or((80, 24));
        rows.saturating_sub(2) // Reserve 2 lines for status bar
    }

    /// Print a tool call preview (for Plan mode - not executed)
    pub fn plan_tool_preview(&self, name: &str, args: &str) {
        if self.use_colors {
            println!(
                "{} {}({}) - {}",
                "[PLAN]".cyan().bold(),
                name.yellow(),
                args.dimmed(),
                "Non execute".cyan().italic()
            );
        } else {
            println!("[PLAN] {}({}) - Non execute", name, args);
        }
    }

    /// Print mode change notification
    pub fn mode_changed(&self, mode: ExecutionMode) {
        let mode_str = match mode {
            ExecutionMode::Plan => {
                if self.use_colors {
                    mode.display_name().cyan().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
            ExecutionMode::Edit => {
                if self.use_colors {
                    mode.display_name().green().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
        };

        let description = match mode {
            ExecutionMode::Plan => "exploration seulement, outils non executes",
            ExecutionMode::Edit => "execution autorisee",
        };

        if self.use_colors {
            println!("{} {} - {}", "→".cyan(), mode_str, description.dimmed());
        } else {
            println!("-> {} - {}", mode_str, description);
        }
    }

    // ============================================================
    // Fixed Panel Methods (Zone fixe en bas de l'ecran)
    // ============================================================

    /// Initialize the fixed panel: draw initial state at the bottom
    pub fn init_fixed_panel(&self, mode: ExecutionMode) {
        let mut stdout = io::stdout();
        let (_, rows) = terminal::size().unwrap_or((80, 24));

        // Set scroll region: lines 1 to (rows - FIXED_PANEL_LINES) (1-indexed)
        let scroll_bottom = rows.saturating_sub(FIXED_PANEL_LINES);
        set_scroll_region(1, scroll_bottom);

        // Move to top of scroll region
        let _ = execute!(stdout, MoveTo(0, 0));

        // Draw the fixed panel at the bottom
        self.draw_fixed_panel_full(mode, "", false, &TokenStats::default());

        // Move cursor to end of scroll area (just above fixed panel)
        let _ = execute!(stdout, MoveTo(0, scroll_bottom.saturating_sub(1)));
        let _ = stdout.flush();
    }

    /// Redraw the fixed panel after terminal resize
    pub fn redraw_fixed_panel(&self) {
        let mut stdout = io::stdout();
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        // Recalculate and re-apply scroll region
        let scroll_bottom = rows.saturating_sub(FIXED_PANEL_LINES);
        set_scroll_region(1, scroll_bottom);

        // Clear and redraw the entire fixed panel area
        for i in 0..FIXED_PANEL_LINES {
            let row = rows.saturating_sub(FIXED_PANEL_LINES) + i;
            let _ = execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine));
        }

        // Redraw separators
        let separator = if self.use_colors {
            "\u{2500}".repeat(cols as usize).dimmed().to_string()
        } else {
            "-".repeat(cols as usize)
        };
        let tool_separator_row = rows.saturating_sub(6);
        let separator1_row = rows.saturating_sub(4);
        let separator2_row = rows.saturating_sub(2);

        let _ = execute!(stdout, MoveTo(0, tool_separator_row));
        print!("{}", separator);
        let _ = execute!(stdout, MoveTo(0, separator1_row));
        print!("{}", separator);
        let _ = execute!(stdout, MoveTo(0, separator2_row));
        print!("{}", separator);

        // Redraw work line (idle state)
        let work_row = rows.saturating_sub(5);
        let _ = execute!(stdout, MoveTo(0, work_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        let idle_label = if self.use_colors {
            "  Pret".green().bold().to_string()
        } else {
            "  Pret".to_string()
        };
        print!("{}", idle_label);

        // Redraw prompt
        let prompt_row = rows.saturating_sub(3);
        let _ = execute!(stdout, MoveTo(0, prompt_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        print!("{} ", ">".green().bold());

        // Move cursor back to scroll area
        let _ = execute!(stdout, MoveTo(0, scroll_bottom.saturating_sub(1)));
        let _ = stdout.flush();
    }

    /// Draw the complete fixed panel
    /// Layout (8 lines):
    ///   Line 1: Tool zone line 1
    ///   Line 2: Tool zone line 2
    ///   Line 3: Separator (tool zone bottom)
    ///   Line 4: Work line (spinner + tokens)
    ///   Line 5: Separator
    ///   Line 6: Prompt > input
    ///   Line 7: Separator
    ///   Line 8: Status (mode + shortcuts)
    fn draw_fixed_panel_full(&self, mode: ExecutionMode, input: &str, working: bool, tokens: &TokenStats) {
        let mut stdout = io::stdout();
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        // Calculate positions for each line (8 lines total)
        let tool_row1 = rows.saturating_sub(8);                      // Line 1: tool zone line 1
        let tool_row2 = rows.saturating_sub(7);                      // Line 2: tool zone line 2
        let tool_separator_row = rows.saturating_sub(6);             // Line 3: tool zone separator
        let work_row = rows.saturating_sub(5);                       // Line 4: work/spinner
        let separator1_row = rows.saturating_sub(4);                 // Line 5: separator
        let prompt_row = rows.saturating_sub(3);                     // Line 6: prompt
        let separator2_row = rows.saturating_sub(2);                 // Line 7: separator
        let status_row = rows.saturating_sub(1);                     // Line 8: status

        let separator = if self.use_colors {
            "─".repeat(cols as usize).dimmed().to_string()
        } else {
            "-".repeat(cols as usize)
        };

        // Line 1: Tool zone line 1 (empty initially)
        let _ = execute!(stdout, MoveTo(0, tool_row1));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));

        // Line 2: Tool zone line 2 (empty initially)
        let _ = execute!(stdout, MoveTo(0, tool_row2));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));

        // Line 3: Tool zone separator
        let _ = execute!(stdout, MoveTo(0, tool_separator_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        print!("{}", separator);

        // Line 4: Work line (spinner + tokens)
        let _ = execute!(stdout, MoveTo(0, work_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_work_line_content(working, "", tokens, cols);

        // Line 5: Separator
        let _ = execute!(stdout, MoveTo(0, separator1_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        print!("{}", separator);

        // Line 6: Prompt line
        let _ = execute!(stdout, MoveTo(0, prompt_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_prompt_line_content(input);

        // Line 7: Separator
        let _ = execute!(stdout, MoveTo(0, separator2_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        print!("{}", separator);

        // Line 8: Status line
        let _ = execute!(stdout, MoveTo(0, status_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_status_line_content(mode, cols);

        let _ = stdout.flush();
    }

    /// Update only the work line (spinner + message + tokens + optional duration)
    pub fn update_work_line(&self, working: bool, message: &str, tokens: &TokenStats) {
        let mut stdout = io::stdout();
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let work_row = rows.saturating_sub(5); // Line 4 of fixed panel (after tool zone)

        let _ = execute!(stdout, SavePosition);
        let _ = execute!(stdout, MoveTo(0, work_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_work_line_content(working, message, tokens, cols);
        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Update work line with elapsed time display
    pub fn update_work_line_with_time(&self, working: bool, message: &str, tokens: &TokenStats, elapsed_secs: f64) {
        let mut stdout = io::stdout();
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let work_row = rows.saturating_sub(5);

        let _ = execute!(stdout, SavePosition);
        let _ = execute!(stdout, MoveTo(0, work_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_work_line_content_with_time(working, message, tokens, cols, Some(elapsed_secs));
        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Draw work line content (helper)
    fn draw_work_line_content(&self, working: bool, message: &str, tokens: &TokenStats, cols: u16) {
        self.draw_work_line_content_with_time(working, message, tokens, cols, None);
    }

    /// Draw work line content with optional elapsed time
    fn draw_work_line_content_with_time(&self, working: bool, message: &str, tokens: &TokenStats, cols: u16, elapsed: Option<f64>) {
        if working {
            // Active: show spinner + message + tokens
            let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() / 80) as usize;
            let spinner = spinner_chars[frame % spinner_chars.len()];

            let msg = if message.is_empty() { "Reflexion..." } else { message };

            // Build time display
            let time_str = match elapsed {
                Some(secs) if secs >= 1.0 => format!(" ({:.0}s)", secs),
                Some(secs) => format!(" ({:.0}ms)", secs * 1000.0),
                None => String::new(),
            };

            let token_info = format!(
                "↓ {} tokens{}",
                TokenStats::format_count(tokens.received),
                time_str,
            );

            if self.use_colors {
                // Calculate spacing
                let left_part = format!("  {} {}", spinner, msg);
                let right_part = &token_info;
                let spacing = cols as usize - left_part.len() - right_part.len() - 2;
                print!(
                    "  {} {}{}{}",
                    spinner.cyan(),
                    msg.dimmed(),
                    " ".repeat(spacing.max(1)),
                    token_info.dimmed()
                );
            } else {
                let left_part = format!("  {} {}", spinner, msg);
                let right_part = &token_info;
                let spacing = cols as usize - left_part.len() - right_part.len() - 2;
                print!("  {} {}{}{}", spinner, msg, " ".repeat(spacing.max(1)), token_info);
            }
        } else {
            // Inactive: show ready status + session total
            let ready_str = "  Pret";
            let session_info = format!("Session: {} tokens", TokenStats::format_count(tokens.session_total));
            let spacing = cols as usize - ready_str.len() - session_info.len() - 2;

            if self.use_colors {
                print!("{}{}{}", ready_str.green(), " ".repeat(spacing.max(1)), session_info.dimmed());
            } else {
                print!("{}{}{}", ready_str, " ".repeat(spacing.max(1)), session_info);
            }
        }
    }

    /// Update only the prompt line
    pub fn update_prompt_line(&self, input: &str) {
        let mut stdout = io::stdout();
        let (_, rows) = terminal::size().unwrap_or((80, 24));
        let prompt_row = rows.saturating_sub(3); // Line 3 of fixed panel

        let _ = execute!(stdout, SavePosition);
        let _ = execute!(stdout, MoveTo(0, prompt_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_prompt_line_content(input);
        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Draw prompt line content (helper)
    fn draw_prompt_line_content(&self, input: &str) {
        if self.use_colors {
            print!("{} {}", ">".green().bold(), input);
        } else {
            print!("> {}", input);
        }
    }

    /// Update only the status line
    pub fn update_status_line(&self, mode: ExecutionMode) {
        let mut stdout = io::stdout();
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let status_row = rows.saturating_sub(1);

        let _ = execute!(stdout, SavePosition);
        let _ = execute!(stdout, MoveTo(0, status_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        self.draw_status_line_content(mode, cols);
        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Draw status line content (helper)
    fn draw_status_line_content(&self, mode: ExecutionMode, cols: u16) {
        let mode_str = match mode {
            ExecutionMode::Plan => {
                if self.use_colors {
                    mode.display_name().cyan().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
            ExecutionMode::Edit => {
                if self.use_colors {
                    mode.display_name().green().bold().to_string()
                } else {
                    mode.display_name().to_string()
                }
            }
        };

        let shortcuts = "Maj+Tab: mode | /aide";

        if self.use_colors {
            // Mode string has ANSI codes, approximate visible length
            let mode_visible_len = mode.display_name().len();
            let spacing = cols as usize - mode_visible_len - shortcuts.len() - 4;
            print!("  {}{}{}", mode_str, " ".repeat(spacing.max(1)), shortcuts.dimmed());
        } else {
            let left_len = 2 + mode_str.len();
            let spacing = cols as usize - left_len - shortcuts.len() - 2;
            print!("  {}{}{}", mode_str, " ".repeat(spacing.max(1)), shortcuts);
        }
    }

    /// Redraw the entire fixed panel (with full state)
    pub fn redraw_fixed_panel_full(&self, mode: ExecutionMode, input: &str, working: bool, tokens: &TokenStats) {
        self.draw_fixed_panel_full(mode, input, working, tokens);
    }

    /// Clean up the fixed panel (clear lines)
    pub fn cleanup_fixed_panel(&self) {
        let mut stdout = io::stdout();
        let (_, rows) = terminal::size().unwrap_or((80, 24));

        // Reset scroll region to full screen
        reset_scroll_region();

        // Clear the fixed panel lines
        for i in 0..FIXED_PANEL_LINES {
            let row = rows.saturating_sub(FIXED_PANEL_LINES - i);
            let _ = execute!(stdout, MoveTo(0, row));
            let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        }

        // Move cursor to end
        let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(FIXED_PANEL_LINES)));
        let _ = stdout.flush();
    }

    /// Move cursor to the prompt input position
    pub fn move_to_prompt_input(&self, input_len: usize) {
        let mut stdout = io::stdout();
        let (_, rows) = terminal::size().unwrap_or((80, 24));
        let prompt_row = rows.saturating_sub(3); // Line 3 of fixed panel
        // Position after "> "
        let col = 2 + input_len as u16;
        let _ = execute!(stdout, MoveTo(col, prompt_row));
        let _ = stdout.flush();
    }

    /// Ensure scroll region is correctly set (re-apply after terminal operations)
    pub fn ensure_scroll_region(&self) {
        let mut stdout = io::stdout();
        let (_, rows) = terminal::size().unwrap_or((80, 24));

        // Re-apply scroll region: lines 1 to (rows - FIXED_PANEL_LINES) (1-indexed)
        let scroll_bottom = rows.saturating_sub(FIXED_PANEL_LINES);
        set_scroll_region(1, scroll_bottom);

        // Ensure cursor is within the scroll region
        let _ = execute!(stdout, MoveTo(0, scroll_bottom.saturating_sub(1)));
        let _ = stdout.flush();
    }

    // ============================================================
    // Tool Zone Methods (Zone fixe pour les interactions tools)
    // ============================================================

    /// Update the tool zone with up to 2 lines of content
    /// This zone is used for tool prompts, status messages, and confirmations
    pub fn update_tool_zone(&self, line1: Option<&str>, line2: Option<&str>) {
        let mut stdout = io::stdout();
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        // Tool zone starts at rows - 8 (first line of fixed panel)
        let tool_row1 = rows.saturating_sub(8);
        let tool_row2 = rows.saturating_sub(7);
        let separator_row = rows.saturating_sub(6);

        let _ = execute!(stdout, SavePosition);

        // Line 1 of tool zone
        let _ = execute!(stdout, MoveTo(0, tool_row1));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        if let Some(text) = line1 {
            print!("{}", text);
        }

        // Line 2 of tool zone
        let _ = execute!(stdout, MoveTo(0, tool_row2));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        if let Some(text) = line2 {
            print!("{}", text);
        }

        // Separator below tool zone
        let _ = execute!(stdout, MoveTo(0, separator_row));
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        if self.use_colors {
            print!("{}", "─".repeat(cols as usize).dimmed());
        } else {
            print!("{}", "-".repeat(cols as usize));
        }

        let _ = execute!(stdout, RestorePosition);
        let _ = stdout.flush();
    }

    /// Clear the tool zone (set both lines to empty)
    pub fn clear_tool_zone(&self) {
        self.update_tool_zone(None, None);
    }
}

/// Format markdown-like text for terminal
pub fn format_markdown(text: &str) -> String {
    let mut result = String::new();
    let mut in_code_block = false;

    for line in text.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            result.push_str(&line.dimmed().to_string());
        } else if in_code_block {
            result.push_str(&format!("  {}", line));
        } else if line.starts_with("# ") {
            result.push_str(&line[2..].bold().to_string());
        } else if line.starts_with("## ") {
            result.push_str(&line[3..].bold().to_string());
        } else if line.starts_with("- ") || line.starts_with("* ") {
            result.push_str(&format!("  • {}", &line[2..]));
        } else if line.starts_with("> ") {
            result.push_str(&format!("│ {}", &line[2..].dimmed()));
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }

    result
}
