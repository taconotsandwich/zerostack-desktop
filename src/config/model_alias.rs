//! Add a model alias without rewriting unrelated configuration or replacing an
//! existing alias. TOML editing preserves comments and feature-specific keys.
use std::io::{self, Write};
use std::path::Path;

use super::QuickModelConfig;

pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("Enter a name for this model.");
    }
    if name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("Use up to 64 bytes of letters, numbers, dots, dashes or underscores.");
    }
    if matches!(name, "main" | "subagent" | "subagents" | "refresh") {
        return Err("That name is reserved by the model menu. Choose another name.");
    }
    Ok(())
}

pub(crate) fn save(path: &Path, name: &str, model: &QuickModelConfig) -> io::Result<()> {
    validate_name(name).map_err(io::Error::other)?;
    let original = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let text = insert_alias(
        original.as_deref().unwrap_or(""),
        path.extension().and_then(|e| e.to_str()),
        name,
        model,
    )?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid configuration path"))?;
    std::fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = path.with_extension(format!("alias-{}-{stamp}.tmp", std::process::id()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        let current = match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if current != original {
            return Err(io::Error::other(
                "Configuration changed while saving. Try again.",
            ));
        }
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

fn insert_alias(
    text: &str,
    extension: Option<&str>,
    name: &str,
    model: &QuickModelConfig,
) -> io::Result<String> {
    let duplicate = || {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Alias '{name}' already exists. Choose another name."),
        )
    };
    if extension == Some("toml") {
        let mut doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(io::Error::other)?;
        let aliases = doc
            .entry("quick_models")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| io::Error::other("quick_models must be a table"))?;
        if aliases.contains_key(name) {
            return Err(duplicate());
        }
        let value = toml::to_string(model)
            .map_err(io::Error::other)?
            .parse::<toml_edit::DocumentMut>()
            .map_err(io::Error::other)?
            .into_table()
            .into_inline_table();
        aliases.insert(name, toml_edit::Item::Value(value.into()));
        Ok(doc.to_string())
    } else if extension == Some("json") {
        let mut doc: serde_json::Value = if text.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(text).map_err(io::Error::other)?
        };
        let root = doc
            .as_object_mut()
            .ok_or_else(|| io::Error::other("Configuration must be an object"))?;
        let aliases = root
            .entry("quick_models")
            .or_insert(serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::other("quick_models must be an object"))?;
        if aliases.contains_key(name) {
            return Err(duplicate());
        }
        aliases.insert(
            name.into(),
            serde_json::to_value(model).map_err(io::Error::other)?,
        );
        serde_json::to_string_pretty(&doc).map_err(io::Error::other)
    } else {
        let mut doc: serde_yaml_ng::Value = if text.is_empty() {
            serde_yaml_ng::Value::Mapping(Default::default())
        } else {
            serde_yaml_ng::from_str(text).map_err(io::Error::other)?
        };
        let root = doc
            .as_mapping_mut()
            .ok_or_else(|| io::Error::other("Configuration must be a mapping"))?;
        let aliases = root
            .entry(serde_yaml_ng::Value::String("quick_models".into()))
            .or_insert(serde_yaml_ng::Value::Mapping(Default::default()))
            .as_mapping_mut()
            .ok_or_else(|| io::Error::other("quick_models must be a mapping"))?;
        let key = serde_yaml_ng::Value::String(name.into());
        if aliases.contains_key(&key) {
            return Err(duplicate());
        }
        aliases.insert(
            key,
            serde_yaml_ng::to_value(model).map_err(io::Error::other)?,
        );
        serde_yaml_ng::to_string(&doc).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> QuickModelConfig {
        toml::from_str("provider = 'gateway'\nmodel = 'new-id'\ncontext_window = 128000\ninput_token_cost = 0.4\n").unwrap()
    }
    #[test]
    fn toml_alias_preserves_comments_unknown_sections_and_inline_tables() {
        for source in [
            "# user comment\nprovider = 'main'\n[future]\nsetting = 123 # keep this\n",
            "# inline aliases\nquick_models = { old = { provider = 'p', model = 'old' } }\n",
        ] {
            let saved = insert_alias(source, Some("toml"), "fast.v2", &model()).unwrap();
            assert!(saved.contains(source.lines().next().unwrap()));
            if source.contains("# keep this") {
                assert!(saved.contains("setting = 123 # keep this"));
            }
            let value: toml::Value = toml::from_str(&saved).unwrap();
            assert_eq!(
                value["quick_models"]["fast.v2"]["model"].as_str(),
                Some("new-id")
            );
            assert_eq!(
                value["quick_models"]["fast.v2"]["context_window"].as_integer(),
                Some(128000)
            );
            assert_eq!(
                insert_alias(&saved, Some("toml"), "fast.v2", &model())
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::AlreadyExists
            );
        }
    }
    #[test]
    fn yaml_and_json_preserve_unknown_keys_and_reject_duplicates() {
        for (extension, source) in [
            ("yaml", "future: {setting: 123}\n"),
            ("json", r#"{"future":{"setting":123}}"#),
        ] {
            let saved = insert_alias(source, Some(extension), "fast", &model()).unwrap();
            let value: serde_json::Value = serde_yaml_ng::from_str(&saved).unwrap();
            assert_eq!(value["future"]["setting"], 123);
            assert_eq!(value["quick_models"]["fast"]["provider"], "gateway");
            assert_eq!(
                insert_alias(&saved, Some(extension), "fast", &model())
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::AlreadyExists
            );
        }
    }
    #[test]
    fn failed_save_never_changes_the_config_and_new_config_is_readable() {
        let root =
            std::env::temp_dir().join(format!("zerostack-alias-save-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        std::fs::write(&path, "[broken").unwrap();
        assert!(save(&path, "fast", &model()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[broken");
        std::fs::remove_file(&path).unwrap();
        save(&path, "fast", &model()).unwrap();
        let original = std::fs::read_to_string(&path).unwrap();
        let cfg: super::super::Config = toml::from_str(&original).unwrap();
        assert_eq!(cfg.quick_models.unwrap()["fast"].model, "new-id");
        assert!(save(&path, "fast", &model()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        for name in ["", "two words", "main", "subagents", "refresh", "/bad"] {
            assert!(save(&path, name, &model()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
