use super::app::{App, Message, Panel};
use super::{components, layout, style, worker::Operation};
use crate::engine::models::{AgentRole, ModelChoice};
use iced::widget::{column, row, scrollable, text};
use iced::{Element, Fill, Task};

pub(super) struct Models {
    pub role: AgentRole,
    query: String,
    provider: String,
    custom_id: String,
    alias: String,
    selected: Option<ModelChoice>,
    choices: Vec<ModelChoice>,
    notice: String,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            role: AgentRole::Main,
            query: String::new(),
            provider: String::new(),
            custom_id: String::new(),
            alias: String::new(),
            selected: None,
            choices: Vec::new(),
            notice: String::new(),
        }
    }
}
impl Models {
    pub(super) fn favorite_saved(
        &mut self,
        name: &str,
        session: &crate::session::Session,
        cfg: &crate::config::Config,
    ) {
        self.choices = crate::engine::models::model_choices(cfg, &session.provider, &session.model);
        self.notice = format!("Saved favorite: {name}");
    }
}
#[derive(Debug, Clone)]
pub(super) enum Event {
    Open,
    Role(AgentRole),
    Search(String),
    Select(ModelChoice),
    Provider(String),
    Custom(String),
    Alias(String),
    Apply,
    SaveFavorite,
}

fn matches(choice: &ModelChoice, query: &str) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        choice.label,
        choice.key.provider,
        choice.key.id,
        choice.aliases.join(" ")
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| haystack.contains(word))
}
impl App {
    pub(super) fn update_models(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Open => {
                let cfg = crate::config::load().0;
                let (provider, model) = self
                    .snapshot
                    .as_ref()
                    .map(|s| (s.session.provider.as_str(), s.session.model.as_str()))
                    .unwrap_or(("anthropic", ""));
                self.models.choices = crate::engine::models::model_choices(&cfg, provider, model);
                self.models.provider = provider.into();
                self.models.notice.clear();
                self.models.query.clear();
                self.models.selected = None;
                self.models.custom_id.clear();
                self.models.alias.clear();
                self.panel = Some(Panel::Models);
            }
            Event::Role(role) => self.models.role = role,
            Event::Search(query) => self.models.query = query,
            Event::Select(choice) => {
                self.models.provider = choice.key.provider.clone();
                self.models.custom_id = choice.key.id.clone();
                self.models.selected = Some(choice);
            }
            Event::Provider(value) => {
                self.models.provider = value;
                self.models.selected = None;
            }
            Event::Custom(value) => {
                self.models.custom_id = value;
                self.models.selected = None;
            }
            Event::Alias(value) => self.models.alias = value,
            Event::Apply if !self.busy => {
                let selection = format!(
                    "{}::{}",
                    self.models.provider.trim(),
                    self.models.custom_id.trim()
                );
                if self.models.provider.trim().is_empty() || self.models.custom_id.trim().is_empty()
                {
                    self.error = "Select a model or enter its provider and ID.".into();
                    return Task::none();
                }
                self.panel = None;
                return self.dispatch(Operation::SelectRoleModel {
                    role: self.models.role,
                    selection,
                });
            }
            Event::SaveFavorite if !self.busy => {
                return self.dispatch(Operation::SaveQuickModel {
                    name: self.models.alias.trim().into(),
                    provider: self.models.provider.trim().into(),
                    model: self.models.custom_id.trim().into(),
                });
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn models_view(&self) -> Element<'_, Message> {
        let roles = [
            ("Main", AgentRole::Main),
            #[cfg(feature = "subagents")]
            ("Subagents", AgentRole::Subagent),
            #[cfg(feature = "advisor")]
            ("Advisor", AgentRole::Advisor),
        ]
        .into_iter()
        .fold(row![].spacing(layout::SM), |row, (label, role)| {
            row.push(components::action(
                label,
                Some(Message::Models(Event::Role(role))),
            ))
        });
        let mut choices = column![].spacing(layout::XS);
        let filtered: Vec<_> = self
            .models
            .choices
            .iter()
            .filter(|choice| matches(choice, &self.models.query))
            .collect();
        for choice in filtered.iter().take(100) {
            let selected = self
                .models
                .selected
                .as_ref()
                .is_some_and(|s| s.key == choice.key);
            choices = choices.push(
                components::action(
                    format!(
                        "{}{} · {}",
                        if selected { "✓ " } else { "" },
                        choice.label,
                        choice.key.provider
                    ),
                    Some(Message::Models(Event::Select((*choice).clone()))),
                )
                .width(Fill),
            );
        }
        let current = self
            .snapshot
            .as_ref()
            .and_then(|s| s.role_models.get(self.models.role.option()))
            .map(String::as_str)
            .unwrap_or("Not selected");
        let mut content = column![
            text("Models").size(style::TITLE),
            roles,
            text(format!("{:?} · current: {current}", self.models.role)).size(style::LABEL),
            text("Applies to subsequent calls in this conversation.").size(style::CAPTION),
            components::field("Search models or favorites…", &self.models.query)
                .on_input(|v| Message::Models(Event::Search(v))),
            text(format!("{} matches · showing up to 100", filtered.len())).size(style::CAPTION),
            iced::widget::container(scrollable(choices)).height(220),
        ]
        .spacing(layout::SM);
        if let Some(choice) = &self.models.selected {
            content = content.push(text(choice.detail()).size(style::CAPTION));
        }
        content = content
            .push(text("Custom model or selected model").size(style::LABEL))
            .push(
                components::field("Provider", &self.models.provider)
                    .on_input(|v| Message::Models(Event::Provider(v))),
            )
            .push(
                components::field("Model ID", &self.models.custom_id)
                    .on_input(|v| Message::Models(Event::Custom(v))),
            )
            .push(components::action(
                "Use model",
                (!self.busy && !self.models.custom_id.trim().is_empty())
                    .then_some(Message::Models(Event::Apply)),
            ))
            .push(
                row![
                    components::field("Favorite name (optional)", &self.models.alias)
                        .on_input(|v| Message::Models(Event::Alias(v))),
                    components::action(
                        "Save favorite",
                        (!self.busy
                            && !self.models.alias.trim().is_empty()
                            && !self.models.custom_id.trim().is_empty())
                        .then_some(Message::Models(Event::SaveFavorite))
                    )
                ]
                .spacing(layout::SM),
            );
        content
            .push(text(&self.models.notice).size(style::CAPTION))
            .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_matches_provider_id_label_and_alias_together() {
        let mut choice = ModelChoice::new("custom", "claude-sonnet");
        choice.aliases.push("daily".into());
        assert!(matches(&choice, "DAILY custom"));
        assert!(matches(&choice, "sonnet"));
        assert!(!matches(&choice, "daily missing"));
    }
}

#[cfg(test)]
mod favorite_tests {
    use super::*;
    #[test]
    fn saved_favorites_refresh_the_open_picker() {
        let cfg: crate::config::Config = serde_json::from_value(serde_json::json!({"quick_models": {"everyday": {"provider": "anthropic", "model": "test-model"}}})).unwrap();
        let session = crate::session::Session::new("anthropic", "test-model", 1000, "");
        let mut models = Models::default();
        models.favorite_saved("everyday", &session, &cfg);
        assert!(
            models
                .choices
                .iter()
                .any(|choice| choice.key.id == "test-model"
                    && choice.aliases.iter().any(|alias| alias == "everyday"))
        );
        assert_eq!(models.notice, "Saved favorite: everyday");
    }
}
