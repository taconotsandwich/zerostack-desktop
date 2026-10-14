//! Provider configuration remains available even when an ACP session cannot start.
use super::app::{App, Message, Panel};
use super::{components, layout, style};
use crate::config::setup::{CredentialUpdate, ProviderSetup};
use crate::config::{self, Config, CustomProviderConfig};
use iced::widget::{column, row, text};
use iced::{Element, Fill, Task};

pub(super) struct Setup {
    config: Config,
    pub name: String,
    pub model: String,
    key: String,
    pub endpoint: String,
    pub protocol: String,
    pub message: String,
    pub busy: bool,
}

#[derive(Debug, Clone)]
pub(super) enum Event {
    Provider(String),
    Model(String),
    Key(String),
    Endpoint(String),
    Protocol(String),
    Test,
    Save,
    Tested(Result<Vec<String>, String>),
    Saved(Result<(), String>),
}

impl Setup {
    pub fn new(config: Config) -> Self {
        let name = config
            .provider
            .as_deref()
            .unwrap_or("anthropic")
            .to_string();
        let model = config.model.as_deref().unwrap_or("").to_string();
        let mut setup = Self {
            config,
            name,
            model,
            key: String::new(),
            endpoint: String::new(),
            protocol: "openai".into(),
            message: String::new(),
            busy: false,
        };
        setup.select_provider(setup.name.clone());
        setup
    }
    fn select_provider(&mut self, name: String) {
        self.name = name;
        self.key.clear();
        self.endpoint.clear();
        if let Some(custom) = self.config.custom_providers_map().get(&self.name) {
            self.endpoint = custom.base_url.clone();
            self.protocol = custom.provider_type.to_string();
        }
    }
    fn request(&self) -> ProviderSetup {
        let existing = self.config.custom_providers_map();
        let custom = if self.endpoint.trim().is_empty() {
            None
        } else {
            Some(CustomProviderConfig {
                provider_type: self.protocol.clone().into(),
                base_url: self.endpoint.trim().into(),
                api_key_env: existing.get(&self.name).and_then(|c| c.api_key_env.clone()),
                danger_accept_invalid_certs: None,
                api_style: existing.get(&self.name).and_then(|c| c.api_style.clone()),
                headers: existing
                    .get(&self.name)
                    .map(|c| c.headers.clone())
                    .unwrap_or_default(),
                timeout_secs: existing.get(&self.name).and_then(|c| c.timeout_secs),
                model: None,
            })
        };
        ProviderSetup {
            name: self.name.trim().into(),
            model: self.model.trim().into(),
            credential: if self.key.is_empty() {
                CredentialUpdate::Keep
            } else {
                CredentialUpdate::Replace(self.key.clone())
            },
            custom,
        }
    }
}
impl App {
    pub(super) fn update_setup(&mut self, event: Event) -> Task<Message> {
        if self.setup.busy && !matches!(event, Event::Tested(_) | Event::Saved(_)) {
            return Task::none();
        }
        match event {
            Event::Provider(value) => self.setup.select_provider(value),
            Event::Model(value) => self.setup.model = value,
            Event::Key(value) => self.setup.key = value,
            Event::Endpoint(value) => self.setup.endpoint = value,
            Event::Protocol(value) => self.setup.protocol = value,
            Event::Test => {
                let request = self.setup.request();
                let cfg = self.setup.config.clone();
                self.setup.busy = true;
                self.setup.message = "Connecting…".into();
                return Task::perform(
                    async move {
                        request
                            .test_connection(&cfg)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    |result| Message::Setup(Event::Tested(result)),
                );
            }
            Event::Save => {
                let request = self.setup.request();
                if let Err(error) = request.prepare(&self.setup.config) {
                    self.setup.message = error.to_string();
                    return Task::none();
                }
                self.setup.busy = true;
                self.setup.message = "Saving…".into();
                let path = config::config_file_path();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            request.save(&path).map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    |result| Message::Setup(Event::Saved(result)),
                );
            }
            Event::Tested(result) => {
                self.setup.busy = false;
                self.setup.message = match result {
                    Ok(models) => format!(
                        "Connected · {} models available. Completion access depends on your account.",
                        models.len()
                    ),
                    Err(error) => error,
                };
            }
            Event::Saved(result) => {
                self.setup.busy = false;
                self.setup.key.clear();
                match result {
                    Ok(()) => {
                        self.setup.config = config::load().0;
                        self.setup.message = "Saved. New conversations use these defaults.".into();
                        if self.snapshot.is_none() && !self.busy {
                            return self.open_project(self.project.clone().into(), None);
                        }
                    }
                    Err(error) => self.setup.message = error,
                }
            }
        }
        Task::none()
    }
    pub(super) fn setup_view(&self) -> Element<'_, Message> {
        let state = &self.setup;
        let mut providers: Vec<String> = crate::setup::builtin_provider_names()
            .iter()
            .map(|v| v.to_string())
            .chain(state.config.custom_providers_map().into_keys())
            .collect();
        providers.sort();
        providers.dedup();
        column![
            text("Connect a provider").size(style::TITLE),
            text("Choose a provider or enter a name and API endpoint for a custom connection.")
                .size(style::LABEL),
            components::choice(
                providers,
                Some(state.name.clone()),
                |v| Message::Setup(Event::Provider(v)),
                Fill
            ),
            components::field("Provider name", &state.name)
                .on_input(|v| Message::Setup(Event::Provider(v))),
            components::field("Model ID", &state.model)
                .on_input(|v| Message::Setup(Event::Model(v))),
            components::field("API key · leave blank to keep existing", &state.key)
                .secure(true)
                .on_input(|v| Message::Setup(Event::Key(v))),
            text("Environment credentials take precedence over saved keys.").size(style::CAPTION),
            components::field("Custom API URL (optional)", &state.endpoint)
                .on_input(|v| Message::Setup(Event::Endpoint(v))),
            components::choice(
                vec!["openai".into(), "anthropic".into(), "gemini".into()],
                Some(state.protocol.clone()),
                |v| Message::Setup(Event::Protocol(v)),
                Fill
            ),
            row![
                components::action(
                    "Test connection",
                    (!state.busy).then_some(Message::Setup(Event::Test))
                ),
                components::action(
                    "Save provider",
                    (!state.busy).then_some(Message::Setup(Event::Save))
                )
            ]
            .spacing(layout::SM),
            text(&state.message).size(style::LABEL),
            components::action("Back to settings", Some(Message::Show(Panel::Settings)))
        ]
        .spacing(layout::MD)
        .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_keeps_credentials_until_explicitly_replaced() {
        let mut setup = Setup::new(Config::default());
        assert!(matches!(setup.request().credential, CredentialUpdate::Keep));
        setup.key = "secret".into();
        assert!(matches!(
            setup.request().credential,
            CredentialUpdate::Replace(_)
        ));
        setup.select_provider("openai".into());
        assert!(setup.key.is_empty());
        setup.endpoint = "http://localhost:8000/v1".into();
        assert_eq!(setup.request().custom.unwrap().base_url, setup.endpoint);
    }
}
