//! Fonctions de prompt standalone pour les tools
//! Écrivent directement dans la zone tool fixe (lignes rows-8 et rows-7)

use colored::*;
use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType},
};
use std::io::{self, Write};


/// Position de la zone tool dans le panel fixe
const TOOL_LINE1_OFFSET: u16 = 8; // rows - 8
const TOOL_LINE2_OFFSET: u16 = 7; // rows - 7
const TOOL_SEP_OFFSET: u16 = 6; // rows - 6

/// Guard pour raw mode
struct RawModeGuard;
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Affiche un statut dans la ligne 1 de la zone tool
pub fn tool_status(message: &str) {
    let mut stdout = io::stdout();
    let (_, rows) = terminal::size().unwrap_or((80, 24));

    let _ = execute!(stdout, SavePosition);
    let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(TOOL_LINE1_OFFSET)));
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));
    print!("{}", message);
    let _ = execute!(stdout, RestorePosition);
    let _ = stdout.flush();
}

/// Affiche un message dans la ligne 2 de la zone tool
pub fn tool_prompt_line(message: &str) {
    let mut stdout = io::stdout();
    let (_, rows) = terminal::size().unwrap_or((80, 24));

    let _ = execute!(stdout, SavePosition);
    let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(TOOL_LINE2_OFFSET)));
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));
    print!("{}", message);
    let _ = execute!(stdout, RestorePosition);
    let _ = stdout.flush();
}

/// Efface la zone tool (les 2 lignes)
pub fn tool_clear() {
    let mut stdout = io::stdout();
    let (cols, rows) = terminal::size().unwrap_or((80, 24));

    let _ = execute!(stdout, SavePosition);

    // Ligne 1
    let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(TOOL_LINE1_OFFSET)));
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));

    // Ligne 2
    let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(TOOL_LINE2_OFFSET)));
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));

    // Redessiner le séparateur
    let _ = execute!(stdout, MoveTo(0, rows.saturating_sub(TOOL_SEP_OFFSET)));
    let _ = execute!(stdout, Clear(ClearType::CurrentLine));
    print!("{}", "─".repeat(cols as usize).dimmed());

    let _ = execute!(stdout, RestorePosition);
    let _ = stdout.flush();
}

/// Prompt oui/non dans la zone tool. Retourne true pour oui.
pub fn tool_prompt_yesno(status: &str, question: &str) -> io::Result<bool> {
    tool_status(status);
    tool_prompt_line(&format!("{} {}", question, "[o/n]".dimmed()));

    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    let result = loop {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            ..
        }) = event::read()?
        {
            if kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                tool_clear();
                return Ok(false);
            }
            match code {
                KeyCode::Char('o') | KeyCode::Char('O') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    break true
                }
                KeyCode::Char('n') | KeyCode::Char('N') => break false,
                KeyCode::Enter => break true,
                KeyCode::Esc => break false,
                _ => continue,
            }
        }
    };

    let now = chrono::Local::now().format("%H:%M:%S");
    println!("{} 🤖 {} → {}", now, question, if result { "oui" } else { "non" });
    tool_clear();
    Ok(result)
}

/// Prompt choix multiple (1, 2, 3...) dans la zone tool
pub fn tool_prompt_choice(status: &str, choices: &[&str]) -> io::Result<usize> {
    tool_status(status);

    // Construire la ligne des choix: "[1] premier [2] second ..."
    let choices_str: String = choices
        .iter()
        .enumerate()
        .map(|(i, c)| format!("[{}] {}", i + 1, c))
        .collect::<Vec<_>>()
        .join("  ");
    tool_prompt_line(&choices_str);

    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    let result = loop {
        if let Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            ..
        }) = event::read()?
        {
            if kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                tool_clear();
                return Ok(1); // Default to first choice on cancel
            }
            match code {
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let n = c.to_digit(10).unwrap_or(0) as usize;
                    if n >= 1 && n <= choices.len() {
                        break n;
                    }
                }
                KeyCode::Enter => break 1, // Default to first
                KeyCode::Esc => break 1,
                _ => continue,
            }
        }
    };

    let now = chrono::Local::now().format("%H:%M:%S");
    println!("{} 🤖 {} → {}", now, status, choices.get(result - 1).unwrap_or(&&"?"));
    tool_clear();
    Ok(result)
}

/// Affiche une progression dans la zone tool (pour OCR, etc.)
pub fn tool_progress(status: &str, current: usize, total: usize, elapsed_secs: f64) {
    let percent = if total > 0 {
        (current * 100) / total
    } else {
        0
    };
    let mins = (elapsed_secs as u64) / 60;
    let secs = (elapsed_secs as u64) % 60;

    // Blink effect
    let blink = if ((elapsed_secs * 2.0) as u64) % 2 == 0 {
        "🔴"
    } else {
        "⬛"
    };

    let line1 = format!("{} {}", blink, status.cyan());
    let line2 = format!("{}/{}  ({}%)  {:02}:{:02}", current, total, percent, mins, secs);

    tool_status(&line1);
    tool_prompt_line(&line2);
}

/// Affiche un message de fin (succès)
pub fn tool_done(message: &str) {
    tool_status(&format!("🟢 {}", message.green()));
    tool_prompt_line("");
}

/// Prompt pour saisir du texte libre (ex: numéros de pages)
pub fn tool_prompt_text(prompt: &str) -> io::Result<String> {
    tool_prompt_line(&format!("{} ", prompt));

    // Lire caractère par caractère pour afficher dans la zone tool
    let mut input = String::new();
    terminal::enable_raw_mode()?;
    let _guard = RawModeGuard;

    loop {
        if let Event::Key(KeyEvent { code, kind, .. }) = event::read()? {
            if kind != crossterm::event::KeyEventKind::Press {
                continue;
            }
            match code {
                KeyCode::Enter => break,
                KeyCode::Esc => {
                    input.clear();
                    break;
                }
                KeyCode::Backspace => {
                    input.pop();
                    tool_prompt_line(&format!("{} {}", prompt, input));
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    tool_prompt_line(&format!("{} {}", prompt, input));
                }
                _ => {}
            }
        }
    }

    let now = chrono::Local::now().format("%H:%M:%S");
    println!("{} 🤖 {} → {}", now, prompt, input);
    tool_clear();
    Ok(input)
}
