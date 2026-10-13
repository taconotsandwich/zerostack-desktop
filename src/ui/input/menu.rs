//! Modal command navigation. Queries and pending actions never use the chat
//! buffer, so selecting a setting leaves both the draft and its caret intact.
use compact_str::CompactString;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{InputEditor, Picker};
use crate::ui::commands;
use crate::ui::pickers::{
    list::ListPicker,
    models::{
        ModelChoice, ModelKey, ModelTarget, ModelTargetPicker, ModelsPicker, SaveModelPicker,
        add_saved_alias,
    },
};

impl InputEditor {
    pub fn set_model_choices(&mut self, mut choices: Vec<ModelChoice>, current: ModelKey) {
        if self.current_model.as_ref() != Some(&current) {
            self.recent_models.retain(|key| key != &current);
            self.recent_models.insert(0, current.clone());
            self.recent_models.truncate(8);
        }
        for saved in &self.saved_models {
            add_saved_alias(&mut choices, saved);
        }
        self.model_choices = choices;
        self.current_model = Some(current);
    }

    #[cfg(feature = "subagents")]
    pub fn set_subagent_model_choices(&mut self, mut choices: Vec<ModelChoice>, current: ModelKey) {
        if self.current_subagent_model.as_ref() != Some(&current) {
            self.recent_subagent_models.retain(|key| key != &current);
            self.recent_subagent_models.insert(0, current.clone());
            self.recent_subagent_models.truncate(8);
        }
        for saved in &self.saved_models {
            add_saved_alias(&mut choices, saved);
        }
        self.subagent_model_choices = choices;
        self.current_subagent_model = Some(current);
    }

    pub fn start_model_picker(&mut self) {
        self.menu_parents.clear();
        self.open_model_menu();
    }

    fn open_model_menu(&mut self) {
        let targets = vec![
            (
                ModelTarget::Conversation,
                self.current_model.clone().unwrap_or_else(empty_model),
            ),
            #[cfg(feature = "subagents")]
            (
                ModelTarget::Subagent,
                self.current_subagent_model
                    .clone()
                    .unwrap_or_else(empty_model),
            ),
        ];
        if targets.len() == 1 {
            self.open_model_picker(ModelTarget::Conversation);
        } else {
            self.picker = Some(Picker::ModelTarget(ModelTargetPicker::new(targets)));
        }
    }

    pub fn start_model_picker_for(&mut self, target: ModelTarget) {
        self.start_model_picker();
        if let Some(Picker::ModelTarget(mut picker)) = self.picker.take() {
            picker.list.selected = picker
                .targets
                .iter()
                .position(|(t, _)| *t == target)
                .unwrap_or(0);
            self.menu_parents.push(Picker::ModelTarget(picker));
        }
        self.open_model_picker(target);
    }

    fn open_model_picker(&mut self, target: ModelTarget) {
        let (choices, current, recent) = match target {
            ModelTarget::Conversation => (
                &self.model_choices,
                &self.current_model,
                &self.recent_models,
            ),
            #[cfg(feature = "subagents")]
            ModelTarget::Subagent => (
                &self.subagent_model_choices,
                &self.current_subagent_model,
                &self.recent_subagent_models,
            ),
        };
        let current = current.clone().unwrap_or(ModelKey {
            provider: String::new(),
            id: String::new(),
        });
        let mut picker = ModelsPicker::new(choices.clone(), current, recent.clone());
        picker.target = target;
        self.picker = Some(Picker::Models(picker));
    }

    pub fn take_model_selection(&mut self) -> Option<(ModelTarget, ModelChoice)> {
        self.model_selection.take()
    }

    pub fn take_model_save(&mut self) -> Option<(String, ModelChoice)> {
        self.model_save.take()
    }

    pub fn saved_model_alias(&self, name: &str) -> Option<ModelChoice> {
        self.saved_models
            .iter()
            .find(|choice| choice.aliases.iter().any(|alias| alias == name))
            .cloned()
    }

    pub fn finish_model_save(&mut self, result: Result<(), String>) {
        let Some(Picker::SaveModel(mut form)) = self.picker.take() else {
            return;
        };
        if let Err(error) = result {
            form.error = Some(error);
            self.picker = Some(Picker::SaveModel(form));
            return;
        }
        let name = form.input.query.trim().to_string();
        let mut saved = form.choice;
        saved.label = name.clone();
        saved.aliases = vec![name];
        self.saved_models.push(saved.clone());
        add_saved_alias(&mut self.model_choices, &saved);
        #[cfg(feature = "subagents")]
        add_saved_alias(&mut self.subagent_model_choices, &saved);
        self.menu_back();
        if let Some(Picker::Models(picker)) = self.picker.as_mut() {
            picker.add_saved(&saved);
        }
    }

    fn menu_back(&mut self) {
        self.picker = self.menu_parents.pop();
    }

    fn enter_command(&mut self, command: &'static commands::Command) {
        if let Some(parent) = self.picker.take() {
            self.menu_parents.push(parent);
        }
        if command.name == "/model" {
            self.open_model_menu();
            return;
        }
        let items: Vec<String> = match command.name {
            "/prompt" => with_default(&self.prompt_names),
            "/theme" => with_default(&self.theme_names),
            "/mode" => strings(&["standard", "restrictive", "readonly", "guarded", "yolo"]),
            "/queue" => strings(&["ls", "pop", "clear"]),
            "/editsys" => strings(&["similarity", "hashedit"]),
            "/toggle" => strings(&["todo on", "todo off"]),
            _ => Vec::new(),
        };
        let mut picker = ListPicker::new();
        picker.set_items(items);
        picker.activate();
        self.picker = Some(Picker::Argument(picker, command.name));
    }

    fn submit_menu(&mut self, command: String) -> bool {
        self.picker = None;
        self.menu_parents.clear();
        self.menu_submission = Some(CompactString::new(command));
        // Hand off to the normal submission path without submitting the draft.
        false
    }

    pub(super) fn handle_menu_key(&mut self, mut key: KeyEvent) -> bool {
        if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT) {
            key.code = KeyCode::BackTab;
        }
        let backspace = key.code == KeyCode::Backspace
            || key.code == KeyCode::Char('\x08')
            || (key.code == KeyCode::Char('h') && key.modifiers.contains(KeyModifiers::CONTROL));
        if key.code == KeyCode::Esc {
            self.menu_back();
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            if let Some(Picker::Models(p)) = self.picker.as_ref()
                && let Some(choice) = p.selected_model().cloned()
            {
                self.menu_parents.push(self.picker.take().unwrap());
                self.picker = Some(Picker::SaveModel(SaveModelPicker::new(choice)));
            }
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
            match self.picker.as_mut() {
                Some(Picker::Command(p) | Picker::Argument(p, _)) => p.set_query(String::new()),
                Some(Picker::Models(p)) => p.set_query(String::new()),
                Some(Picker::ModelTarget(p)) => p.list.set_query(String::new()),
                Some(Picker::SaveModel(p)) => {
                    p.input.set_query(String::new());
                    p.error = None;
                }
                _ => {}
            }
            return true;
        }
        if matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End | KeyCode::Delete
        ) {
            let (query, cursor) = match self.picker.as_mut() {
                Some(Picker::Command(p) | Picker::Argument(p, _)) => (&mut p.query, &mut p.cursor),
                Some(Picker::Models(p)) => (&mut p.query, &mut p.cursor),
                Some(Picker::ModelTarget(p)) => (&mut p.list.query, &mut p.list.cursor),
                Some(Picker::SaveModel(p)) => (&mut p.input.query, &mut p.input.cursor),
                _ => return false,
            };
            match key.code {
                KeyCode::Left => *cursor = cursor.saturating_sub(1),
                KeyCode::Right => *cursor = (*cursor + 1).min(query.chars().count()),
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = query.chars().count(),
                KeyCode::Delete => {
                    if let Some((pos, _)) = query.char_indices().nth(*cursor) {
                        query.remove(pos);
                        let query = query.clone();
                        let cursor = *cursor;
                        match self.picker.as_mut() {
                            Some(Picker::Command(p) | Picker::Argument(p, _)) => {
                                p.set_query(query);
                                p.cursor = cursor;
                            }
                            Some(Picker::Models(p)) => {
                                p.set_query(query);
                                p.cursor = cursor;
                            }
                            Some(Picker::ModelTarget(p)) => {
                                p.list.set_query(query);
                                p.list.cursor = cursor;
                            }
                            Some(Picker::SaveModel(p)) => {
                                p.input.set_query(query);
                                p.input.cursor = cursor;
                                p.error = None;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return true;
        }
        // Modal editing cannot accidentally run actions or type into the draft.
        if !backspace
            && key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            || key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT)
        {
            return true;
        }
        match self.picker.as_mut() {
            Some(Picker::Command(p)) => {
                if backspace {
                    if p.query.is_empty() {
                        self.menu_back();
                    } else {
                        p.backspace();
                    }
                    return true;
                }
                match key.code {
                    KeyCode::Char(' ') => {
                        if p.cursor == p.query.chars().count()
                            && let Some(cmd) =
                                commands::find(&format!("/{}", p.query.trim_start_matches('/')))
                        {
                            self.enter_command(cmd);
                        } else {
                            p.char_input(' ');
                        }
                    }
                    KeyCode::Char(c) => p.char_input(c),
                    KeyCode::Up | KeyCode::BackTab => p.select_prev(),
                    KeyCode::Down => p.select_next(),
                    KeyCode::Tab => {
                        if let Some(name) = p.selected_name().map(str::to_string) {
                            p.set_query(name.trim_start_matches('/').into());
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(cmd) = p.selected_name().and_then(commands::find) {
                            let guided = ModelTarget::from_command(cmd.name).is_some()
                                || matches!(
                                    cmd.name,
                                    "/model"
                                        | "/prompt"
                                        | "/theme"
                                        | "/mode"
                                        | "/queue"
                                        | "/editsys"
                                        | "/toggle"
                                );
                            if cmd.arguments.is_empty()
                                || (!guided && !cmd.arguments.starts_with('<'))
                            {
                                return self.submit_menu(cmd.name.into());
                            }
                            self.enter_command(cmd);
                        }
                    }
                    _ => {}
                }
            }
            Some(Picker::Argument(p, command)) => {
                if backspace {
                    if p.query.is_empty() {
                        self.menu_back();
                    } else {
                        p.backspace();
                    }
                    return true;
                }
                match key.code {
                    KeyCode::Char(c) => p.char_input(c),
                    KeyCode::Up | KeyCode::BackTab => p.select_prev(),
                    KeyCode::Down => p.select_next(),
                    KeyCode::Tab => {
                        if let Some(value) = p.selected_name().map(str::to_string) {
                            p.set_query(value);
                        }
                    }
                    KeyCode::Enter => {
                        let value = p.selected_name().unwrap_or(&p.query).trim();
                        let cmd = commands::find(command).expect("argument command");
                        if !value.is_empty() || !cmd.arguments.starts_with('<') {
                            let text = format!("{command} {value}").trim_end().to_string();
                            return self.submit_menu(text);
                        }
                    }
                    _ => {}
                }
            }
            Some(Picker::ModelTarget(p)) => {
                if backspace {
                    if p.list.query.is_empty() {
                        self.menu_back();
                    } else {
                        p.list.backspace();
                    }
                    return true;
                }
                let target = p.selected_target();
                match key.code {
                    KeyCode::Enter | KeyCode::Char(' ')
                        if target.is_some()
                            && (key.code == KeyCode::Enter
                                || ModelTarget::from_name(&p.list.query) == target) =>
                    {
                        let target = target.unwrap();
                        self.menu_parents.push(self.picker.take().unwrap());
                        self.open_model_picker(target);
                    }
                    KeyCode::Char(c) => p.list.char_input(c),
                    KeyCode::Up | KeyCode::BackTab => p.list.select_prev(),
                    KeyCode::Down => p.list.select_next(),
                    KeyCode::Tab => {
                        if let Some(target) = target {
                            p.list.set_query(target.name().into());
                        }
                    }
                    _ => {}
                }
            }
            Some(Picker::SaveModel(p)) => {
                if backspace {
                    if p.input.query.is_empty() {
                        self.menu_back();
                    } else {
                        p.input.backspace();
                        p.error = None;
                    }
                    return true;
                }
                match key.code {
                    KeyCode::Char(c) => {
                        p.input.char_input(c);
                        p.error = None;
                    }
                    KeyCode::Enter => {
                        let name = p.input.query.trim();
                        if let Err(error) = crate::config::model_alias::validate_name(name) {
                            p.error = Some(error.into());
                        } else {
                            self.model_save = Some((name.into(), p.choice.clone()));
                        }
                    }
                    _ => {}
                }
            }
            Some(Picker::Models(p)) => {
                if backspace {
                    if p.query.is_empty() {
                        self.menu_back();
                    } else {
                        p.backspace();
                    }
                    return true;
                }
                match key.code {
                    KeyCode::Char(c) => p.char_input(c),
                    KeyCode::Up | KeyCode::BackTab => p.select_prev(),
                    KeyCode::Down => p.select_next(),
                    KeyCode::Tab => {
                        if let Some(choice) = p.selected_model() {
                            let value = format!("{}::{}", choice.key.provider, choice.key.id);
                            p.set_query(value);
                        }
                    }
                    KeyCode::Enter if p.query.trim() == "refresh" => {
                        let text = format!("{} refresh", p.target.command());
                        return self.submit_menu(text);
                    }
                    KeyCode::Enter => {
                        if let Some(choice) = p.selected_model().cloned() {
                            self.model_selection = Some((p.target, choice));
                            self.picker = None;
                            self.menu_parents.clear();
                        }
                    }
                    _ => {}
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn paste_into_menu(&mut self, text: &str) -> bool {
        if !matches!(
            self.picker,
            Some(
                Picker::Command(_)
                    | Picker::Argument(..)
                    | Picker::Models(_)
                    | Picker::ModelTarget(_)
                    | Picker::SaveModel(_)
            )
        ) {
            return false;
        }
        // Paste is query input, never a sequence of Enter actions.
        for c in text.chars() {
            self.handle_menu_key(KeyEvent::new(
                KeyCode::Char(if c.is_control() { ' ' } else { c }),
                KeyModifiers::NONE,
            ));
        }
        true
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}
fn with_default(values: &[String]) -> Vec<String> {
    let mut values = values.to_vec();
    values.sort();
    if !values.iter().any(|value| value == "default") {
        values.insert(0, "default".into());
    }
    values
}

fn empty_model() -> ModelKey {
    ModelKey {
        provider: String::new(),
        id: String::new(),
    }
}
