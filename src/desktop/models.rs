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
    custom_open: bool,
    favorite_open: bool,
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
            custom_open: false,
            favorite_open: false,
        }
    }
}
impl Models {
    fn select(&mut self, choice: ModelChoice) {
        self.provider = choice.key.provider.clone();
        self.custom_id = choice.key.id.clone();
        self.selected = Some(choice);
        self.custom_open = false;
        self.favorite_open = false;
        self.alias.clear();
        self.notice.clear();
    }

    fn can_apply(&self) -> bool {
        !self.provider.trim().is_empty() && !self.custom_id.trim().is_empty()
    }

    fn can_save_favorite(&self) -> bool {
        self.favorite_open && self.can_apply() && !self.alias.trim().is_empty()
    }

    pub(super) fn favorite_saved(
        &mut self,
        name: &str,
        session: &crate::session::Session,
        cfg: &crate::config::Config,
    ) {
        self.choices = crate::engine::models::model_choices(cfg, &session.provider, &session.model);
        self.notice = format!("Saved favorite: {name}");
        self.favorite_open = false;
        self.alias.clear();
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
    ToggleCustom,
    ToggleFavorite,
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
                self.models.custom_open = false;
                self.models.favorite_open = false;
                self.panel = Some(Panel::Models);
                return iced::widget::operation::focus("model-search");
            }
            Event::Role(role) => self.models.role = role,
            Event::Search(query) => self.models.query = query,
            Event::Select(choice) => {
                self.models.select(choice);
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
            Event::ToggleCustom => {
                self.models.custom_open = !self.models.custom_open;
                if self.models.custom_open {
                    return iced::widget::operation::focus("model-provider");
                }
            }
            Event::ToggleFavorite => {
                self.models.favorite_open = !self.models.favorite_open;
                if self.models.favorite_open {
                    return iced::widget::operation::focus("favorite-name");
                }
            }
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
            Event::SaveFavorite if !self.busy && self.models.can_save_favorite() => {
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
            row.push(
                components::action(label, Some(Message::Models(Event::Role(role)))).style(
                    move |theme, status| {
                        style::selected_action(theme, status, self.models.role == role)
                    },
                ),
            )
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
            roles,
            text(format!("{:?} · current: {current}", self.models.role)).size(style::LABEL),
            text("Applies to subsequent calls in this conversation.").size(style::CAPTION),
            components::field("Search models or favorites…", &self.models.query)
                .id("model-search")
                .on_input(|v| Message::Models(Event::Search(v))),
            iced::widget::container(scrollable(choices)).height(
                (filtered.len().min(100) as f32 * layout::CONTROL_HEIGHT)
                    .clamp(layout::CONTROL_HEIGHT, 220.0)
            ),
        ]
        .spacing(layout::SM);
        if filtered.len() > 100 {
            content = content.push(
                text(format!(
                    "Showing 100 of {} models · refine your search",
                    filtered.len()
                ))
                .size(style::CAPTION),
            );
        } else if filtered.is_empty() {
            content = content.push(
                text("No models match. Try another search or add a custom model.")
                    .size(style::CAPTION),
            );
        }
        if let Some(choice) = &self.models.selected {
            content = content.push(text(choice.detail()).size(style::CAPTION));
        }
        content = content.push(components::action(
            if self.models.custom_open {
                "Hide custom model"
            } else {
                "Custom model…"
            },
            Some(Message::Models(Event::ToggleCustom)),
        ));
        if self.models.custom_open {
            content = content
                .push(text("Provider").size(style::CAPTION))
                .push(
                    components::field("Provider", &self.models.provider)
                        .id("model-provider")
                        .on_input(|v| Message::Models(Event::Provider(v))),
                )
                .push(text("Model ID").size(style::CAPTION))
                .push(
                    components::field("Model ID", &self.models.custom_id)
                        .on_input(|v| Message::Models(Event::Custom(v))),
                );
        }
        content = content.push(
            row![
                components::action(
                    "Use model",
                    (!self.busy && self.models.can_apply())
                        .then_some(Message::Models(Event::Apply))
                ),
                components::icon_button(
                    style::Icon::Favorite,
                    "Save as favorite",
                    (!self.busy && self.models.can_apply())
                        .then_some(Message::Models(Event::ToggleFavorite))
                ),
            ]
            .spacing(layout::SM)
            .align_y(iced::Center),
        );
        if self.models.favorite_open {
            content = content.push(
                row![
                    components::field("Favorite name", &self.models.alias)
                        .id("favorite-name")
                        .on_input(|v| Message::Models(Event::Alias(v)))
                        .on_submit(Message::Models(Event::SaveFavorite)),
                    components::action(
                        "Save",
                        (!self.busy && self.models.can_save_favorite())
                            .then_some(Message::Models(Event::SaveFavorite))
                    ),
                    components::icon_button(
                        style::Icon::Close,
                        "Cancel favorite",
                        Some(Message::Models(Event::ToggleFavorite))
                    ),
                ]
                .spacing(layout::SM)
                .align_y(iced::Center),
            );
        }
        content
            .push(text(&self.models.notice).size(style::CAPTION))
            .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choosing_a_model_closes_advanced_fields_and_discards_stale_favorite_names() {
        let mut models = Models {
            custom_open: true,
            favorite_open: true,
            alias: "old".into(),
            ..Models::default()
        };
        models.select(ModelChoice::new("provider", "model"));
        assert!(models.can_apply());
        assert!(!models.custom_open);
        assert!(!models.favorite_open);
        assert!(models.alias.is_empty());
        assert!(!models.can_save_favorite());
        models.favorite_open = true;
        models.alias = "daily".into();
        assert!(models.can_save_favorite());
        models.provider = " ".into();
        assert!(!models.can_apply());
        assert!(!models.can_save_favorite());
    }

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
