//! One model selector for slash commands and Alt+M. Identity is always
//! (provider, model ID), never a display label or a quick-model alias.
use super::fuzzy_score;

pub use crate::engine::models::{ModelChoice, ModelKey, ModelTarget};

/// The first screen of /model shows each target and its current model.
pub struct ModelTargetPicker {
    pub list: super::list::ListPicker,
    pub targets: Vec<(ModelTarget, ModelKey)>,
}

impl ModelTargetPicker {
    pub fn new(targets: Vec<(ModelTarget, ModelKey)>) -> Self {
        let mut list = super::list::ListPicker::new();
        list.set_items(
            targets
                .iter()
                .map(|(target, _)| target.name().into())
                .collect(),
        );
        list.activate();
        Self { list, targets }
    }

    pub fn selected_target(&self) -> Option<ModelTarget> {
        self.list.selected_name().and_then(ModelTarget::from_name)
    }

    pub fn row(&self, name: &str) -> String {
        let Some((target, current)) = self
            .targets
            .iter()
            .find(|(target, _)| target.name() == name)
        else {
            return name.into();
        };
        let label = match target {
            ModelTarget::Conversation => "Main agent",
            #[cfg(feature = "subagents")]
            ModelTarget::Subagent => "Subagents",
        };
        format!("{label} · {} · {}", current.id, current.provider)
    }
}

pub struct SaveModelPicker {
    pub input: super::list::ListPicker,
    pub choice: ModelChoice,
    pub error: Option<String>,
}

impl SaveModelPicker {
    pub fn new(choice: ModelChoice) -> Self {
        let mut input = super::list::ListPicker::new();
        input.activate();
        Self {
            input,
            choice,
            error: None,
        }
    }
}

pub fn add_saved_alias(choices: &mut Vec<ModelChoice>, saved: &ModelChoice) {
    if let Some(choice) = choices.iter_mut().find(|choice| choice.key == saved.key) {
        if choice.aliases.is_empty() {
            choice.label = saved.label.clone();
        }
        for alias in &saved.aliases {
            if !choice.aliases.contains(alias) {
                choice.aliases.push(alias.clone());
            }
        }
    } else {
        choices.push(saved.clone());
    }
}

pub struct ModelsPicker {
    pub target: ModelTarget,
    pub active: bool,
    pub query: String,
    pub cursor: usize,
    pub matches: Vec<ModelChoice>,
    pub selected: usize,
    pub current: ModelKey,
    pub recent: Vec<ModelKey>,
    pub busy: bool,
    pub notice: Option<String>,
    choices: Vec<ModelChoice>,
}

impl ModelsPicker {
    pub fn new(choices: Vec<ModelChoice>, current: ModelKey, recent: Vec<ModelKey>) -> Self {
        let mut picker = Self {
            target: ModelTarget::Conversation,
            active: true,
            query: String::new(),
            cursor: 0,
            matches: Vec::new(),
            selected: 0,
            current,
            recent,
            busy: false,
            notice: None,
            choices,
        };
        picker.filter();
        picker
    }

    pub fn add_saved(&mut self, saved: &ModelChoice) {
        let selected = self.selected_model().map(|choice| choice.key.clone());
        add_saved_alias(&mut self.choices, saved);
        self.filter();
        if let Some(selected) = selected {
            self.selected = self
                .matches
                .iter()
                .position(|choice| choice.key == selected)
                .unwrap_or(0);
        }
        self.notice = Some(format!(
            "Saved as {} · available in future sessions",
            saved.label
        ));
    }

    pub fn set_query(&mut self, query: String) {
        self.cursor = query.chars().count();
        self.query = query;
        self.filter();
    }

    pub fn char_input(&mut self, c: char) {
        let pos = self
            .query
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len());
        self.query.insert(pos, c);
        self.cursor += 1;
        self.filter();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let pos = self
                .query
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.query.len());
            self.query.remove(pos);
        }
        self.filter();
    }

    fn filter(&mut self) {
        let query = self.query.trim();
        let search = query.replace("::", " ");
        let mut scored: Vec<_> = self
            .choices
            .iter()
            .filter_map(|choice| {
                let searchable = format!(
                    "{} {} {} {}",
                    choice.label,
                    choice.key.provider,
                    choice.key.id,
                    choice.aliases.join(" ")
                );
                if let Some((provider, _)) = query.split_once("::")
                    && !choice.key.provider.eq_ignore_ascii_case(provider)
                {
                    return None;
                }
                let score = search.split_whitespace().try_fold(0, |sum, word| {
                    fuzzy_score(&searchable, word).map(|score| sum + score)
                })?;
                let score = if choice.key.id.eq_ignore_ascii_case(query)
                    || choice.label.eq_ignore_ascii_case(query)
                    || choice
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(query))
                {
                    score + 10000
                } else {
                    score
                };
                let rank = if choice.key == self.current {
                    0
                } else {
                    self.recent
                        .iter()
                        .position(|key| key == &choice.key)
                        .map(|i| i + 1)
                        .unwrap_or(if choice.aliases.is_empty() {
                            usize::MAX
                        } else {
                            usize::MAX - 1
                        })
                };
                Some((score, rank, choice))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.label.cmp(&b.2.label))
                .then(a.2.key.provider.cmp(&b.2.key.provider))
        });
        self.matches = scored
            .into_iter()
            .map(|(_, _, choice)| choice.clone())
            .collect();
        // A deliberate, labeled escape hatch for new IDs absent from the catalog.
        if self.matches.is_empty()
            && !query.is_empty()
            && !query.contains(char::is_whitespace)
            && query != "refresh"
        {
            let (provider, id) = query
                .split_once("::")
                .unwrap_or((&self.current.provider, query));
            if !provider.is_empty() && !id.is_empty() {
                let mut custom = ModelChoice::new(provider, id);
                custom.label = format!("Use model ID: {id}");
                self.matches.push(custom);
            }
        }
        self.selected = 0;
    }

    pub fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + 1) % self.matches.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            self.selected = (self.selected + self.matches.len() - 1) % self.matches.len();
        }
    }

    pub fn selected_model(&self) -> Option<&ModelChoice> {
        self.matches.get(self.selected)
    }

    pub fn row(&self, choice: &ModelChoice) -> String {
        let group = if choice.key == self.current {
            "Current"
        } else if self.recent.contains(&choice.key) {
            "Recent"
        } else if !choice.aliases.is_empty() {
            "Saved"
        } else {
            "Available"
        };
        format!(
            "{} {} · {} · {group}",
            if choice.key == self.current {
                "✓"
            } else {
                " "
            },
            choice.label,
            choice.key.provider
        )
    }
}
