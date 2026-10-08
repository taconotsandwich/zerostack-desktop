//! What a graphical frontend reads from the engine: docs, colors, memory.

use crate::config::Config;
use crate::engine::Engine;
use crate::provider::AnyAgent;
use crate::sandbox::Sandbox;
use crate::tests::engine_tests::{
    engine_with_turns, isolate_data_dirs, test_cli, test_client, test_context, test_session,
};
use crate::tests::fake_model;

#[test]
fn active_colors_fall_back_to_config_and_prefer_the_selected_theme() {
    isolate_data_dirs();
    let model = fake_model::text_turns(Vec::<Vec<&str>>::new());
    let agent = AnyAgent::Mock(rig::agent::AgentBuilder::new(model).build());
    let cfg = Config {
        colors: Some(crate::config::ColorsConfig {
            chat_background: Some("#202020".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut engine = Engine::new(
        test_cli(),
        cfg,
        test_session(),
        test_context(),
        test_client(),
        None,
        Sandbox::new(false, "bwrap"),
    )
    .with_agent(agent);

    // No theme selected: the config `[colors]` section is the fallback.
    let config_colors = engine.active_colors().expect("config colors");
    assert_eq!(config_colors.chat_background.as_deref(), Some("#202020"));

    let theme = serde_json::json!({
        "chat_background": "#101010",
        "roles": { "agent": "white", "error": "#ff5555" },
    })
    .to_string();
    engine
        .context_mut()
        .themes
        .insert("test-theme".to_string(), theme);
    engine.context_mut().current_theme_name = Some("test-theme".to_string());

    let colors = engine.active_colors().expect("theme colors");
    assert_eq!(colors.chat_background.as_deref(), Some("#101010"));
    assert_eq!(
        colors
            .roles
            .as_ref()
            .and_then(|roles| roles.get("error"))
            .map(String::as_str),
        Some("#ff5555")
    );
}

#[test]
fn read_doc_serves_bundled_docs_and_rejects_traversal() {
    let (engine, _model) = engine_with_turns(vec![]);
    let content = engine.read_doc("GET_STARTED.md").expect("bundled doc");
    assert!(
        content.to_lowercase().contains("zerostack"),
        "unexpected doc content"
    );
    assert!(engine.read_doc("").is_err());
    assert!(engine.read_doc("../secrets.md").is_err());
    assert!(engine.read_doc("missing-file.md").is_err());
}

#[cfg(feature = "memory")]
#[test]
fn memory_editor_path_points_at_memory_md() {
    let (engine, _model) = engine_with_turns(vec![]);
    let path = engine.memory_editor_path();
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("MEMORY.md")
    );
}
