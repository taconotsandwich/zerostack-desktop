use compact_str::CompactString;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::file::FilePicker;
use super::list::ListPicker;

use crate::ui::input::cursor::prev_char_boundary;

pub fn handle_file_key(
    buffer: &mut CompactString,
    cursor: &mut usize,
    picker: &mut FilePicker,
    key: KeyEvent,
) -> bool {
    match key.code {
        KeyCode::Char(c)
            if c == '\x08' || (c == 'h' && key.modifiers.contains(KeyModifiers::CONTROL)) =>
        {
            if picker.cursor > 0 {
                picker.backspace();
                *cursor = prev_char_boundary(buffer, *cursor);
                buffer.remove(*cursor);
            } else {
                let at_pos = buffer.rfind('@');
                if let Some(at) = at_pos {
                    let before: String = buffer.chars().take(at).collect();
                    let after: String = buffer.chars().skip(at + 1).collect();
                    *buffer = format!("{}{}", before, after).into();
                    *cursor = at;
                }
                picker.deactivate();
            }
            true
        }
        KeyCode::Char(c) => {
            picker.char_input(c);
            buffer.insert(*cursor, c);
            *cursor += c.len_utf8();
            true
        }
        KeyCode::Backspace => {
            if picker.cursor > 0 {
                picker.backspace();
                *cursor = prev_char_boundary(buffer, *cursor);
                buffer.remove(*cursor);
                true
            } else {
                let at_pos = buffer.rfind('@');
                if let Some(at) = at_pos {
                    let before: String = buffer.chars().take(at).collect();
                    let after: String = buffer.chars().skip(at + 1).collect();
                    *buffer = format!("{}{}", before, after).into();
                    *cursor = at;
                }
                picker.deactivate();
                true
            }
        }
        KeyCode::Tab => {
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::SHIFT)
            {
                picker.select_prev();
            } else {
                picker.select_next();
            }
            true
        }
        KeyCode::Up => {
            picker.select_prev();
            true
        }
        KeyCode::Down => {
            picker.select_next();
            true
        }
        KeyCode::Enter => {
            if let Some(path) = picker.selected_path() {
                let path_str = path.to_string_lossy().to_string();
                let at_pos = buffer.rfind('@');
                if let Some(at) = at_pos {
                    let before: String = buffer.chars().take(at).collect();
                    let after_offset = at + 1 + picker.query.len();
                    let after: String = buffer.chars().skip(after_offset).collect();
                    let new_len = before.len() + path_str.len();
                    *buffer = format!("{}{}{}", before, path_str, after).into();
                    *cursor = new_len;
                }
            }
            picker.deactivate();
            true
        }
        KeyCode::Esc => {
            let at_pos = buffer.rfind('@');
            if let Some(at) = at_pos {
                let before: String = buffer.chars().take(at).collect();
                let after: String = buffer.chars().skip(at + 1 + picker.query.len()).collect();
                *buffer = format!("{}{}", before, after).into();
                *cursor = at;
            }
            picker.deactivate();
            true
        }
        _ => false,
    }
}

pub fn handle_prefixed_key(
    buffer: &mut CompactString,
    cursor: &mut usize,
    picker: &mut ListPicker,
    prefix: &str,
    key: KeyEvent,
) -> bool {
    let backspace = key.code == KeyCode::Backspace
        || key.code == KeyCode::Char('\x08')
        || (key.code == KeyCode::Char('h') && key.modifiers.contains(KeyModifiers::CONTROL));
    if (!backspace
        && key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT))
        || (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT))
    {
        picker.deactivate();
        return false;
    }
    if backspace {
        if picker.query.is_empty() {
            picker.deactivate();
            return false;
        }
        picker.backspace();
        *buffer = format!("{prefix}{}", picker.query).into();
        *cursor = buffer.len();
        return true;
    }
    match key.code {
        KeyCode::Char(' ') => {
            picker.deactivate();
            false
        }
        KeyCode::Char(c) => {
            picker.char_input(c);
            *buffer = format!("{prefix}{}", picker.query).into();
            *cursor = buffer.len();
            true
        }
        KeyCode::Up | KeyCode::BackTab => {
            picker.select_prev();
            true
        }
        KeyCode::Down => {
            picker.select_next();
            true
        }
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            picker.select_prev();
            true
        }
        KeyCode::Enter | KeyCode::Tab => {
            if let Some(name) = picker.selected_name() {
                *buffer = format!("{prefix}{name}").into();
                *cursor = buffer.len();
            }
            picker.deactivate();
            // Dot prompts are also used as prefixes for one-shot questions.
            key.code == KeyCode::Tab || prefix == "."
        }
        KeyCode::Esc => {
            picker.deactivate();
            true
        }
        _ => {
            picker.deactivate();
            false
        }
    }
}
