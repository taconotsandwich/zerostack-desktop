use crossterm::event::KeyEvent;

use crate::ui::pickers::file::FilePicker;
use crate::ui::pickers::handlers;
use crate::ui::pickers::list::ListPicker;
use crate::ui::pickers::models::{ModelTargetPicker, ModelsPicker, SaveModelPicker};
use crate::ui::pickers::rewind::RewindPicker;
use crate::ui::pickers::switcher::PromptSwitcher;

pub enum Picker {
    File(FilePicker),
    Command(ListPicker),
    Argument(ListPicker, &'static str),
    Prefixed(ListPicker, &'static str),
    Models(ModelsPicker),
    ModelTarget(ModelTargetPicker),
    SaveModel(SaveModelPicker),
    Rewind(RewindPicker),
    PromptSwitcher(PromptSwitcher),
}

impl Picker {
    pub fn active(&self) -> bool {
        match self {
            Picker::File(p) => p.active,
            Picker::Command(p) | Picker::Argument(p, _) => p.active,
            Picker::Prefixed(p, _) => p.active,
            Picker::Models(p) => p.active,
            Picker::ModelTarget(p) => p.list.active,
            Picker::SaveModel(p) => p.input.active,
            Picker::Rewind(p) => p.active(),
            Picker::PromptSwitcher(p) => p.active(),
        }
    }

    pub fn set_monochrome(&mut self, monochrome: bool) {
        match self {
            Picker::File(p) => p.set_monochrome(monochrome),
            Picker::Command(p) | Picker::Argument(p, _) => p.set_monochrome(monochrome),
            Picker::Prefixed(p, _) => p.set_monochrome(monochrome),
            Picker::Models(_) | Picker::ModelTarget(_) | Picker::SaveModel(_) => {}
            Picker::Rewind(p) => p.set_monochrome(monochrome),
            Picker::PromptSwitcher(p) => p.set_monochrome(monochrome),
        }
    }

    pub fn draw(&mut self) -> std::io::Result<()> {
        match self {
            Picker::File(p) => p.draw(),
            Picker::Command(p) | Picker::Argument(p, _) => p.draw(None),
            Picker::Prefixed(p, prefix) => {
                let msg = if *prefix == "/provider " {
                    Some("no matches  (type a registered custom gateway name)")
                } else {
                    None
                };
                p.draw(msg)
            }
            Picker::Models(_) | Picker::ModelTarget(_) | Picker::SaveModel(_) => Ok(()),
            Picker::Rewind(p) => p.draw(),
            Picker::PromptSwitcher(p) => p.draw(),
        }
    }
}

use super::InputEditor;

impl InputEditor {
    pub fn handle_picker_key(&mut self, key: KeyEvent) -> bool {
        let handled = match self.picker.as_mut() {
            Some(Picker::File(p)) => {
                handlers::handle_file_key(&mut self.buffer, &mut self.cursor, p, key)
            }
            Some(
                Picker::Command(_)
                | Picker::Argument(..)
                | Picker::Models(_)
                | Picker::ModelTarget(_)
                | Picker::SaveModel(_),
            ) => self.handle_menu_key(key),
            Some(Picker::Prefixed(p, prefix)) => {
                handlers::handle_prefixed_key(&mut self.buffer, &mut self.cursor, p, prefix, key)
            }
            Some(Picker::Rewind(p)) => p.handle(key),
            Some(Picker::PromptSwitcher(p)) => p.handle(key),
            None => false,
        };
        if handled {
            self.yank_pos = None;
        }
        handled
    }
}
