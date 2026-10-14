//! Provider setup shared by interactive hosts, without starting an agent.
use std::path::Path;
use std::time::Duration;

use super::{Config, CustomProviderConfig};

#[derive(Clone, Default)]
pub enum CredentialUpdate {
    #[default]
    Keep,
    Replace(String),
    Remove,
}

impl std::fmt::Debug for CredentialUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Keep => "Keep",
            Self::Replace(_) => "Replace([redacted])",
            Self::Remove => "Remove",
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProviderSetup {
    pub name: String,
    pub model: String,
    pub credential: CredentialUpdate,
    /// None retains the existing provider definition.
    pub custom: Option<CustomProviderConfig>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SetupError {
    #[error("{0}")]
    Invalid(String),
    #[error("Authentication was rejected. Check the API key and environment overrides.")]
    Authentication,
    #[error("The provider did not answer within 15 seconds. Check its URL and your network.")]
    Timeout,
    #[error("Could not list models. Check the provider URL, network, and model-list API support.")]
    Connection,
}

impl ProviderSetup {
    pub fn prepare(&self, existing: &Config) -> Result<Config, SetupError> {
        if self.name.is_empty()
            || self
                .name
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(SetupError::Invalid(
                "Enter a provider name without spaces.".into(),
            ));
        }
        if self.model.trim().is_empty() || self.model.chars().any(char::is_control) {
            return Err(SetupError::Invalid("Enter a model ID.".into()));
        }
        let mut cfg = existing.clone();
        if let Some(custom) = &self.custom {
            let url = reqwest::Url::parse(&custom.base_url).map_err(|_| {
                SetupError::Invalid("Enter an absolute HTTP or HTTPS API URL.".into())
            })?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(SetupError::Invalid(
                    "Use an HTTP or HTTPS API URL without credentials, a query, or a fragment."
                        .into(),
                ));
            }
            cfg.custom_providers
                .get_or_insert_default()
                .insert(self.name.clone(), custom.clone());
        }
        crate::provider::resolve_provider_config(&self.name, &cfg.custom_providers_map()).map_err(
            |_| {
                SetupError::Invalid(
                    "Choose a supported provider or configure a custom endpoint.".into(),
                )
            },
        )?;
        match &self.credential {
            CredentialUpdate::Keep => {}
            CredentialUpdate::Replace(key) => {
                if key.trim().is_empty() || key.chars().any(char::is_control) {
                    return Err(SetupError::Invalid(
                        "Enter a nonempty API key without line breaks.".into(),
                    ));
                }
                cfg.api_keys
                    .get_or_insert_default()
                    .insert(self.name.clone(), key.trim().into());
            }
            CredentialUpdate::Remove => {
                cfg.api_keys.get_or_insert_default().remove(&self.name);
            }
        }
        cfg.provider = Some(self.name.clone().into());
        cfg.model = Some(self.model.trim().into());
        Ok(cfg)
    }

    /// Probe the model-list endpoint. A successful public catalog response
    /// does not establish permission to run paid completions.
    pub async fn test_connection(&self, existing: &Config) -> Result<Vec<String>, SetupError> {
        let cfg = self.prepare(existing)?;
        let custom = cfg.custom_providers_map();
        let client = crate::provider::create_client(&self.name, None, &custom, cfg.api_keys.as_ref())
            .map_err(|_| SetupError::Invalid("No usable credential was found. Enter an API key or set the provider's environment variable.".into()))?;
        let request = async {
            if custom.contains_key(&self.name) {
                crate::provider::list_models_manual(
                    &self.name,
                    None,
                    &custom,
                    cfg.api_keys.as_ref(),
                )
                .await
            } else {
                client.list_models().await
            }
        };
        let models = tokio::time::timeout(Duration::from_secs(15), request)
            .await
            .map_err(|_| SetupError::Timeout)?
            .map_err(|error| {
                let message = error.to_string();
                if message.contains("401") || message.contains("403") {
                    SetupError::Authentication
                } else {
                    SetupError::Connection
                }
            })?;
        Ok(models
            .into_iter()
            .filter(crate::provider::is_agent_model)
            .map(|m| m.id)
            .collect())
    }

    /// Change only provider setup fields in the user file. Unknown or
    /// feature-gated configuration survives; project overrides are not saved.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut document: serde_json::Value = if path.exists() {
            let text = std::fs::read_to_string(path)?;
            if path.extension().is_some_and(|ext| ext == "toml") {
                toml::from_str(&text)?
            } else {
                serde_yaml_ng::from_str(&text)?
            }
        } else {
            serde_json::json!({})
        };
        let current: Config = serde_json::from_value(document.clone())?;
        let cfg = self.prepare(&current)?;
        let object = document
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Configuration must be an object."))?;
        object.insert("provider".into(), serde_json::json!(cfg.provider));
        object.insert("model".into(), serde_json::json!(cfg.model));
        if !matches!(self.credential, CredentialUpdate::Keep) {
            object.insert(
                "api_keys".into(),
                serde_json::json!(cfg.api_keys.unwrap_or_default()),
            );
        }
        if self.custom.is_some() {
            object.insert(
                "custom_providers".into(),
                serde_json::to_value(cfg.custom_providers.unwrap_or_default())?,
            );
        }
        let text = if path.extension().is_some_and(|ext| ext == "toml") {
            toml::to_string_pretty(&document)?
        } else {
            serde_yaml_ng::to_string(&document)?
        };
        super::load::atomic_config_write(path, &text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> ProviderSetup {
        ProviderSetup {
            name: "ollama".into(),
            model: "local-model".into(),
            credential: CredentialUpdate::Keep,
            custom: None,
        }
    }

    #[test]
    fn setup_rejects_invalid_fields_without_mutating_config() {
        let cfg = Config::default();
        let mut value = setup();
        value.model.clear();
        assert!(value.prepare(&cfg).is_err());
        value.model = "m".into();
        value.name = "unknown".into();
        assert!(value.prepare(&cfg).is_err());
        value.name = "openai".into();
        value.credential = CredentialUpdate::Replace("\n".into());
        assert!(value.prepare(&cfg).is_err());
        assert!(cfg.provider.is_none());
        assert!(!format!("{:?}", CredentialUpdate::Replace("secret".into())).contains("secret"));
    }

    #[test]
    fn setup_preserves_unknown_settings_and_supports_removing_credentials() {
        let root = std::env::temp_dir().join(format!("zs-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        for extension in ["toml", "yaml", "json"] {
            let path = root.join(format!("config.{extension}"));
            let original = if extension == "toml" {
                "future_setting = 42\n[api_keys]\nother = 'keep'\n"
            } else {
                "{\"future_setting\":42,\"api_keys\":{\"other\":\"keep\"}}"
            };
            std::fs::write(&path, original).unwrap();
            let mut value = setup();
            value.credential = CredentialUpdate::Replace("new-key".into());
            value.save(&path).unwrap();
            value.credential = CredentialUpdate::Keep;
            value.save(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("future_setting"));
            assert!(text.contains("new-key"));
            value.credential = CredentialUpdate::Remove;
            value.save(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("keep"));
            assert!(!text.contains("new-key"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn setup_probes_a_custom_endpoint_without_an_agent() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(String::from_utf8_lossy(&bytes[..n]).starts_with("GET /v1/models "));
            let body = r#"{"data":[{"id":"local-model"}]}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let mut value = setup();
        value.name = "test-setup".into();
        value.custom = Some(CustomProviderConfig {
            provider_type: "openai".into(),
            base_url: format!("http://{address}/v1"),
            api_key_env: Some("ZS_TEST_SETUP_UNUSED_KEY".into()),
            danger_accept_invalid_certs: None,
            api_style: None,
            headers: Default::default(),
            timeout_secs: None,
            model: None,
        });
        value.credential = CredentialUpdate::Replace("test-key".into());
        assert_eq!(
            value.test_connection(&Config::default()).await.unwrap(),
            ["local-model"]
        );
        server.await.unwrap();
    }
}
