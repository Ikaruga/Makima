//! Confirmation dialogs for tool execution — Claude Code style

use super::ui::CliUI;
use colored::*;
use crossterm::{
    cursor::MoveUp,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType},
};
use std::io::{self, Write};

/// Guard that ensures raw mode is disabled when dropped
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Result of a confirmation dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    /// User approved this action
    Yes,
    /// User denied this action
    No,
    /// User approved all actions of this type for the session
    Always,
    /// User wants to cancel the entire operation
    Cancel,
}

/// Map tool internal name to display name (Claude Code style)
fn display_name_for(tool_name: &str) -> &str {
    match tool_name {
        "bash" => "Bash command",
        "write_file" => "Write",
        "edit_file" => "Edit",
        "delete" => "Delete",
        "csv_to_docx" => "CsvToDocx",
        "format_liasse_fiscale" => "FormatLiasse",
        "pdf_to_txt" => "PdfToTxt",
        _ => tool_name,
    }
}

/// Redraw the option menu using crossterm cursor movement
fn redraw_options(options: &[&str], selected: usize) {
    let mut stdout = io::stdout();
    // Move up: options + blank line + hint line
    let lines_up = options.len() as u16 + 2;
    let _ = execute!(stdout, MoveUp(lines_up));

    for (i, opt) in options.iter().enumerate() {
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        if i == selected {
            println!("  {} {}", ">".green().bold(), opt.green().bold());
        } else {
            println!("    {}", opt.dimmed());
        }
    }
    // Blank line + hint
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));
    println!();
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));
    println!("  {}", "Esc: annuler \u{00b7} t: toujours autoriser".dimmed());
    let _ = stdout.flush();
}

/// Clean up the confirmation block and replace with a compact summary line
fn cleanup_confirm_block(summary_lines: usize, options_count: usize, tool_name: &str, result: ConfirmResult) {
    let mut stdout = io::stdout();
    // Total lines: blank + header + summary_lines + blank + question + options + blank + hint
    let total_lines = 1 + 1 + summary_lines + 1 + 1 + options_count + 1 + 1;
    let _ = execute!(stdout, MoveUp(total_lines as u16));

    // Clear all lines
    for _ in 0..total_lines {
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        println!();
    }

    // Move back up to write the compact summary
    let _ = execute!(stdout, MoveUp(total_lines as u16));
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));

    let display = display_name_for(tool_name);
    match result {
        ConfirmResult::Yes => {
            println!("  {} {} {}", "\u{2713}".green(), display.cyan().bold(), "\u{2014} Oui".green());
        }
        ConfirmResult::No => {
            println!("  {} {} {}", "\u{2717}".red(), display.cyan().bold(), "\u{2014} Non".red());
        }
        ConfirmResult::Always => {
            println!("  {} {} {}", "\u{2713}".green(), display.cyan().bold(), "\u{2014} Toujours".cyan());
        }
        ConfirmResult::Cancel => {
            println!("  {} {} {}", "\u{2717}".red(), display.cyan().bold(), "\u{2014} Annule".red());
        }
    }

    // Clear remaining lines below
    for _ in 1..total_lines {
        let _ = execute!(stdout, Clear(ClearType::CurrentLine));
        println!();
    }

    // Move cursor back to just after the summary line
    let remaining = total_lines - 1;
    if remaining > 0 {
        let _ = execute!(stdout, MoveUp(remaining as u16));
    }
    let _ = stdout.flush();
}

/// Ask for confirmation before executing a tool (Claude Code style)
pub fn confirm_tool_execution_with_ui(
    _ui: &CliUI,
    tool_name: &str,
    summary: &str,
) -> io::Result<ConfirmResult> {
    let display = display_name_for(tool_name);
    let summary_lines: Vec<&str> = summary.lines().collect();
    let summary_count = summary_lines.len().max(1);

    // 1. Print the confirmation block
    println!();
    println!("  {}", display.cyan().bold());

    // 2. Summary indented
    for line in &summary_lines {
        println!("     {}", line.dimmed());
    }
    if summary_lines.is_empty() {
        println!("     {}", "(pas de details)".dimmed());
    }

    // 3. Question + options
    let options = ["Oui", "Non"];
    let mut selected: usize = 0;

    println!();
    println!("  {} Executer ?", "?".cyan());
    // Draw initial options
    for (i, opt) in options.iter().enumerate() {
        if i == selected {
            println!("  {} {}", ">".green().bold(), opt.green().bold());
        } else {
            println!("    {}", opt.dimmed());
        }
    }
    // Blank + hint
    println!();
    println!("  {}", "Esc: annuler \u{00b7} t: toujours autoriser".dimmed());
    io::stdout().flush()?;

    // 4. Enter raw mode for key input
    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    let result = loop {
        if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
            if kind != KeyEventKind::Press {
                continue;
            }
            // Ctrl+C
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                break ConfirmResult::Cancel;
            }

            match code {
                // Arrow navigation
                KeyCode::Up => {
                    if selected > 0 {
                        selected -= 1;
                        redraw_options(&options, selected);
                    }
                }
                KeyCode::Down => {
                    if selected < options.len() - 1 {
                        selected += 1;
                        redraw_options(&options, selected);
                    }
                }
                // Enter validates current selection
                KeyCode::Enter => {
                    break match selected {
                        0 => ConfirmResult::Yes,
                        _ => ConfirmResult::No,
                    };
                }
                // Direct shortcuts
                KeyCode::Char('o') | KeyCode::Char('O') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    break ConfirmResult::Yes;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => break ConfirmResult::No,
                KeyCode::Char('t') | KeyCode::Char('T') => break ConfirmResult::Always,
                KeyCode::Char('a') | KeyCode::Char('A') => break ConfirmResult::Cancel,
                KeyCode::Esc => break ConfirmResult::Cancel,
                _ => continue,
            }
        }
    };

    // _guard drops here, disabling raw mode

    // 5. Cleanup: replace block with compact summary
    cleanup_confirm_block(summary_count, options.len(), tool_name, result);

    Ok(result)
}

/// Ask for confirmation before executing a tool (legacy, prints to stdout)
pub fn confirm_tool_execution(tool_name: &str, summary: &str) -> io::Result<ConfirmResult> {
    println!();
    println!("{}", "\u{2501}".repeat(60).dimmed());
    println!("{} {}", "Outil:".yellow().bold(), tool_name.cyan());
    println!("{} {}", "Action:".yellow().bold(), summary);
    println!("{}", "\u{2501}".repeat(60).dimmed());
    println!();
    print!(
        "{} {} ",
        "Autoriser?".yellow().bold(),
        "[o]ui / [n]on / [t]oujours / [a]nnuler".dimmed()
    );
    io::stdout().flush()?;

    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    let result = loop {
        if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
            if kind != KeyEventKind::Press {
                continue;
            }
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                println!();
                return Ok(ConfirmResult::Cancel);
            }

            match code {
                KeyCode::Char('o') | KeyCode::Char('O') | KeyCode::Char('y') | KeyCode::Char('Y') => break ConfirmResult::Yes,
                KeyCode::Char('n') | KeyCode::Char('N') => break ConfirmResult::No,
                KeyCode::Char('t') | KeyCode::Char('T') => break ConfirmResult::Always,
                KeyCode::Char('a') | KeyCode::Char('A') => break ConfirmResult::Cancel,
                KeyCode::Enter => break ConfirmResult::Yes,
                KeyCode::Esc => break ConfirmResult::Cancel,
                _ => continue,
            }
        }
    };

    let choice_str = match result {
        ConfirmResult::Yes => "oui".green(),
        ConfirmResult::No => "non".red(),
        ConfirmResult::Always => "toujours".cyan(),
        ConfirmResult::Cancel => "annuler".red(),
    };
    println!("{}", choice_str);

    Ok(result)
}

/// Simple yes/no confirmation (using tool zone if UI available)
pub fn confirm_simple_with_ui(ui: &CliUI, prompt: &str) -> io::Result<bool> {
    ui.update_tool_zone(Some(&format!("{} {}", prompt.yellow(), "[o/n]".dimmed())), None);

    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    let result = loop {
        if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
            if kind != KeyEventKind::Press {
                continue;
            }
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                ui.clear_tool_zone();
                return Ok(false);
            }

            match code {
                KeyCode::Char('o') | KeyCode::Char('O') | KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                KeyCode::Char('n') | KeyCode::Char('N') => break false,
                KeyCode::Enter => break true,
                KeyCode::Esc => break false,
                _ => continue,
            }
        }
    };

    ui.clear_tool_zone();
    Ok(result)
}

/// Simple yes/no confirmation (legacy, prints to stdout)
pub fn confirm_simple(prompt: &str) -> io::Result<bool> {
    print!("{} {} ", prompt.yellow(), "[o/n]".dimmed());
    io::stdout().flush()?;

    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    let result = loop {
        if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
            if kind != KeyEventKind::Press {
                continue;
            }
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                println!();
                return Ok(false);
            }

            match code {
                KeyCode::Char('o') | KeyCode::Char('O') | KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                KeyCode::Char('n') | KeyCode::Char('N') => break false,
                KeyCode::Enter => break true,
                KeyCode::Esc => break false,
                _ => continue,
            }
        }
    };

    println!("{}", if result { "oui".green() } else { "non".red() });

    Ok(result)
}
