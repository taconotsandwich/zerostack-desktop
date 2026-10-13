use std::collections::HashMap;
use std::io::Write;

use crossterm::ExecutableCommand;
use crossterm::cursor::MoveTo;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Color, ResetColor, SetForegroundColor};
use crossterm::terminal::Clear;

use super::super::utils::resolve_color;
use super::{draw_picker_list, fuzzy_score};

/// Outcome of a modal switcher session, read back by the event loop which
/// owns the session and performs the actual switch via the existing
/// `/models` / `/prompt` slash paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitcherOutcome {
    Confirmed(String),
    Cancelled,
}

/// Which switcher produced the outcome, so the event loop knows which slash
/// command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitcherResult {
    Prompt(String),
    Cancelled,
}

/// Immediate-apply switcher over prompt names.
///
/// Single-group with fuzzy filtering (the
/// insertion `/prompt` picker is substring-only). Enter resolves to
/// `Confirmed(name)`; the caller runs `/prompt <name>` directly.
pub struct PromptSwitcher {
    active: bool,
    query: String,
    cursor: usize,
    matches: Vec<String>,
    selected: usize,
    items: Vec<String>,
    /// prompt name -> precomputed detail (`BuiltIn` / `UserFile`, plus mode).
    details: HashMap<String, String>,
    current: Option<String>,
    outcome: Option<SwitcherOutcome>,
    monochrome: bool,
}

impl PromptSwitcher {
    pub fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            cursor: 0,
            matches: Vec::new(),
            selected: 0,
            items: Vec::new(),
            details: HashMap::new(),
            current: None,
            outcome: None,
            monochrome: false,
        }
    }

    pub fn set_monochrome(&mut self, monochrome: bool) {
        self.monochrome = monochrome;
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
    }

    pub fn set_details(&mut self, details: HashMap<String, String>) {
        self.details = details;
    }

    pub fn set_current(&mut self, current: Option<String>) {
        self.current = current;
    }

    pub fn activate(&mut self) {
        self.active = true;
        self.query.clear();
        self.cursor = 0;
        self.selected = 0;
        self.outcome = None;
        self.filter();
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn take_outcome(&mut self) -> Option<SwitcherOutcome> {
        self.outcome.take()
    }

    fn filter(&mut self) {
        let mut scored: Vec<(i32, &String)> = self
            .items
            .iter()
            .filter_map(|n| fuzzy_score(n, &self.query).map(|s| (s, n)))
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        self.matches = scored
            .into_iter()
            .take(50)
            .map(|(_, n)| n.clone())
            .collect();
        self.selected = 0;
    }

    fn char_input(&mut self, c: char) {
        let byte_pos = self
            .query
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len());
        self.query.insert(byte_pos, c);
        self.cursor += 1;
        self.filter();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 && !self.query.is_empty() {
            self.cursor -= 1;
            let byte_pos = self
                .query
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.query.len());
            self.query.remove(byte_pos);
            self.filter();
        }
    }

    fn display_rows(&self) -> Vec<String> {
        self.matches
            .iter()
            .map(|name| {
                let mut row = name.clone();
                if let Some(d) = self.details.get(name) {
                    row.push_str("  ");
                    row.push_str(d);
                }
                if self.current.as_deref() == Some(name.as_str()) {
                    row.push_str("  ● current");
                }
                row
            })
            .collect()
    }

    pub fn draw(&self) -> std::io::Result<()> {
        if !self.active {
            return Ok(());
        }
        let (_cols, rows) = crossterm::terminal::size()?;
        let mut stdout = std::io::stdout();

        let max_items = (rows.saturating_sub(5)).min(10) as usize;
        let list_height = max_items.min(self.matches.len().max(1));
        let top_row = rows.saturating_sub(3).saturating_sub(list_height as u16);

        if rows >= 8 {
            let header_row = top_row.saturating_sub(1);
            stdout.execute(MoveTo(0, header_row))?;
            write!(
                stdout,
                "{}",
                Clear(crossterm::terminal::ClearType::CurrentLine)
            )?;
            write!(
                stdout,
                "{}",
                SetForegroundColor(resolve_color(Color::DarkGrey, self.monochrome))
            )?;
            write!(
                stdout,
                "[Prompts {}]   (Enter applies immediately · Esc cancels)",
                self.items.len()
            )?;
            write!(stdout, "{}", ResetColor)?;
        }

        let rows_display = self.display_rows();
        let empty = if self.items.is_empty() {
            Some("no prompts — /regen-prompts to restore")
        } else {
            None
        };
        draw_picker_list(&rows_display, self.selected, self.monochrome, empty, 5)
    }

    pub fn handle(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT)
        {
            if key.code == KeyCode::Char('h')
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                self.backspace();
            }
            return true;
        }
        match key.code {
            KeyCode::Char('\x08') => {
                self.backspace();
            }
            KeyCode::Char(c) => {
                self.char_input(c);
            }
            KeyCode::Backspace => {
                self.backspace();
            }
            KeyCode::Up => {
                if !self.matches.is_empty() {
                    self.selected = if self.selected == 0 {
                        self.matches.len() - 1
                    } else {
                        self.selected - 1
                    };
                }
            }
            KeyCode::Down => {
                if !self.matches.is_empty() {
                    self.selected = (self.selected + 1) % self.matches.len();
                }
            }
            KeyCode::Enter => {
                if let Some(name) = self.matches.get(self.selected).cloned() {
                    self.outcome = Some(SwitcherOutcome::Confirmed(name));
                } else {
                    self.outcome = Some(SwitcherOutcome::Cancelled);
                }
                self.active = false;
            }
            KeyCode::Esc => {
                self.outcome = Some(SwitcherOutcome::Cancelled);
                self.active = false;
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn char_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn prompt_switcher() -> PromptSwitcher {
        let mut p = PromptSwitcher::new();
        p.set_items(vec![
            "code".to_string(),
            "plan".to_string(),
            "ask".to_string(),
        ]);
        let mut details = HashMap::new();
        details.insert("code".to_string(), "BuiltIn".to_string());
        p.set_details(details);
        p.set_current(Some("code".to_string()));
        p.activate();
        p
    }

    #[test]
    fn prompt_switcher_filters_fuzzy_and_confirms() {
        let mut p = prompt_switcher();
        p.handle(char_key('p'));
        assert_eq!(p.matches, vec!["plan".to_string()]);
        p.handle(key(KeyCode::Enter));
        assert_eq!(
            p.take_outcome(),
            Some(SwitcherOutcome::Confirmed("plan".to_string()))
        );
    }

    #[test]
    fn prompt_switcher_esc_cancels() {
        let mut p = prompt_switcher();
        p.handle(key(KeyCode::Esc));
        assert_eq!(p.take_outcome(), Some(SwitcherOutcome::Cancelled));
        assert!(!p.active());
    }
}
