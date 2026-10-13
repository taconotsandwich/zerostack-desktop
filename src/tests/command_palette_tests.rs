use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::{
    commands,
    input::{InputEditor, Picker},
    pickers::palette::Palette,
};

fn press(editor: &mut InputEditor, code: KeyCode) -> Option<String> {
    key(editor, KeyEvent::new(code, KeyModifiers::NONE))
}

// Follow the app's routing: the picker gets first refusal, then the composer.
fn key(editor: &mut InputEditor, event: KeyEvent) -> Option<String> {
    if editor.picker.as_ref().is_some_and(|p| p.active()) && editor.handle_picker_key(event) {
        return None;
    }
    editor.handle_key(event).map(|s| s.to_string())
}

fn type_text(editor: &mut InputEditor, text: &str) {
    for c in text.chars() {
        press(editor, KeyCode::Char(c));
    }
}

#[test]
fn command_catalog_names_and_aliases_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for cmd in commands::COMMANDS {
        assert!(!cmd.description.is_empty());
        for name in std::iter::once(cmd.name).chain(cmd.aliases.iter().copied()) {
            assert!(name.starts_with('/'));
            assert!(seen.insert(name), "duplicate {name}");
            assert_eq!(commands::find(name).unwrap().name, cmd.name);
        }
    }
}

#[test]
fn command_search_finds_aliases_descriptions_categories_and_fuzzy_names() {
    assert_eq!(commands::search("clear")[0].name, "/new");
    assert_eq!(commands::search("model")[0].name, "/model");
    assert_eq!(commands::search(" /THINKING ")[0].name, "/reasoning");
    assert!(
        commands::search("context files")
            .iter()
            .any(|cmd| cmd.name == "/add")
    );
    assert!(
        commands::search("permissions")
            .iter()
            .any(|cmd| cmd.name == "/mode")
    );
    assert!(
        commands::search("rwd")
            .iter()
            .any(|cmd| cmd.name == "/rewind")
    );
    let all = commands::search("");
    assert_eq!(all[0].name, "/help");
    assert!(all.iter().any(|cmd| cmd.name == "/new"));
    assert!(!all.iter().any(|cmd| cmd.name == "/clear"));
    assert!(commands::search("zzzzzzzz").is_empty());
}

#[test]
fn catalog_only_offers_compiled_features() {
    for (name, enabled) in [
        ("/memory", cfg!(feature = "memory")),
        ("/export", cfg!(feature = "export")),
        ("/hooks", cfg!(feature = "hooks")),
        ("/advisor", cfg!(feature = "advisor")),
        ("/loop", cfg!(feature = "loop")),
        ("/worktree", cfg!(feature = "git-worktree")),
        ("/mcp", cfg!(feature = "mcp")),
        ("/model-subagent", cfg!(feature = "subagents")),
        ("/models-subagent", cfg!(feature = "subagents")),
    ] {
        assert_eq!(commands::find(name).is_some(), enabled, "{name}");
    }
}

#[test]
fn enter_runs_a_command_and_tab_only_completes_the_query() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/und");
    assert!(press(&mut editor, KeyCode::Tab).is_none());
    assert!(editor.buffer.is_empty());
    assert!(matches!(&editor.picker, Some(Picker::Command(p)) if p.query == "undo"));
    assert_eq!(press(&mut editor, KeyCode::Enter).as_deref(), Some("/undo"));
}

#[test]
fn command_search_apply_and_back_preserve_unicode_multiline_draft_and_cursor() {
    let mut editor = InputEditor::new();
    editor.load_text("first\n中文 draft");
    editor.set_cursor(6);
    editor.start_command_picker();
    type_text(&mut editor, "permissions");
    press(&mut editor, KeyCode::Enter);
    assert!(matches!(editor.picker, Some(Picker::Argument(_, "/mode"))));
    type_text(&mut editor, "read");
    press(&mut editor, KeyCode::Esc);
    assert!(matches!(&editor.picker, Some(Picker::Command(p)) if p.query == "permissions"));
    press(&mut editor, KeyCode::Enter);
    type_text(&mut editor, "read");
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/mode readonly")
    );
    assert_eq!(editor.buffer, "first\n中文 draft");
    assert_eq!(editor.cursor, 6);
    editor.start_command_picker();
    type_text(&mut editor, "model");
    press(&mut editor, KeyCode::Esc);
    assert!(editor.picker.is_none());
    assert_eq!(editor.buffer, "first\n中文 draft");
    assert_eq!(editor.cursor, 6);
}

#[test]
fn direct_arguments_and_required_value_entry_stay_in_the_menu() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/rename");
    assert!(press(&mut editor, KeyCode::Enter).is_none());
    assert!(
        press(&mut editor, KeyCode::Enter).is_none(),
        "required argument must not submit empty"
    );
    assert!(matches!(
        editor.picker,
        Some(Picker::Argument(_, "/rename"))
    ));
    type_text(&mut editor, "新しい 会話");
    assert!(editor.buffer.is_empty());
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/rename 新しい 会話")
    );
    type_text(&mut editor, "/help permissions");
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/help permissions")
    );
}

#[test]
fn space_opens_described_choices_and_tab_never_applies() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/mode ");
    let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), 80).unwrap();
    assert!(palette.items.iter().any(|s| s.contains("Read only")));
    type_text(&mut editor, "read");
    assert!(press(&mut editor, KeyCode::Tab).is_none());
    assert!(matches!(&editor.picker, Some(Picker::Argument(p, _)) if p.query == "readonly"));
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/mode readonly")
    );
}

#[test]
fn backspace_at_empty_argument_goes_back_and_root_closes() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/mode ");
    press(&mut editor, KeyCode::Backspace);
    assert!(matches!(&editor.picker, Some(Picker::Command(p)) if p.query == "mode"));
    key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    press(&mut editor, KeyCode::Backspace);
    assert!(editor.picker.is_none());
    assert!(editor.buffer.is_empty());
}

#[test]
fn paste_edits_menu_without_submitting_or_changing_draft() {
    let mut editor = InputEditor::new();
    editor.load_text("draft");
    editor.start_command_picker();
    type_text(&mut editor, "ren");
    editor.handle_paste("ame 中文\nmore".into());
    assert_eq!(editor.buffer, "draft");
    assert!(
        matches!(&editor.picker, Some(Picker::Argument(p, "/rename")) if p.query == "中文 more")
    );
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/rename 中文 more")
    );
    assert_eq!(editor.buffer, "draft");
}

#[test]
fn modified_enter_cannot_run_selection_or_edit_draft() {
    for modifier in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
        let mut editor = InputEditor::new();
        editor.load_text("draft");
        editor.start_command_picker();
        type_text(&mut editor, "undo");
        assert!(key(&mut editor, KeyEvent::new(KeyCode::Enter, modifier)).is_none());
        assert_eq!(editor.buffer, "draft");
        assert!(matches!(editor.picker, Some(Picker::Command(_))));
    }
}

#[test]
fn mouse_and_loaded_text_dismiss_menus_without_leaking_queries() {
    let mut editor = InputEditor::new();
    editor.load_text("draft");
    editor.start_command_picker();
    type_text(&mut editor, "rename");
    editor.set_cursor(1);
    press(&mut editor, KeyCode::Char('é'));
    assert_eq!(editor.buffer, "déraft");
    assert!(editor.picker.is_none());
    editor.start_command_picker();
    editor.load_text("restored request");
    assert!(editor.picker.is_none());
    assert_eq!(editor.buffer, "restored request");
}

#[test]
fn prepending_a_command_or_prompt_preserves_the_existing_draft() {
    for prefix in ["/btw ", ".ask "] {
        let mut editor = InputEditor::new();
        editor.load_text("explain this code");
        editor.set_cursor(0);
        type_text(&mut editor, prefix);
        assert_eq!(editor.buffer.as_str(), format!("{prefix}explain this code"));
        assert!(!editor.picker.as_ref().is_some_and(|p| p.active()));
    }
}

#[test]
fn empty_command_search_cannot_execute_an_unrelated_command() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/zzzzzzzzz");
    assert!(press(&mut editor, KeyCode::Enter).is_none());
    assert!(matches!(editor.picker, Some(Picker::Command(_))));
}

#[test]
fn default_prompt_is_discoverable_even_without_custom_prompts() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/prompt ");
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/prompt default")
    );
}

#[test]
fn model_aliases_open_one_selector_and_selection_keeps_the_draft() {
    use crate::ui::pickers::models::ModelChoice;
    for command in ["/model", "/models", "/provider"] {
        let mut editor = InputEditor::new();
        let choice = ModelChoice::new("gateway", "model-fast");
        editor.set_model_choices(vec![choice.clone()], choice.key.clone());
        editor.load_text("draft");
        editor.start_command_picker();
        type_text(&mut editor, command.trim_start_matches('/'));
        press(&mut editor, KeyCode::Enter);
        #[cfg(feature = "subagents")]
        {
            assert!(matches!(editor.picker, Some(Picker::ModelTarget(_))));
            press(&mut editor, KeyCode::Enter);
        }
        assert!(matches!(editor.picker, Some(Picker::Models(_))));
        type_text(&mut editor, "gateway fast");
        press(&mut editor, KeyCode::Enter);
        assert_eq!(editor.take_model_selection().unwrap().1.key, choice.key);
        assert_eq!(editor.buffer, "draft");
    }
}

#[test]
fn palette_keeps_selection_visible_at_every_height() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/");
    press(&mut editor, KeyCode::Up);
    let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), 80).unwrap();
    for height in 0..30 {
        let rows = palette.rows(height);
        assert!(rows.len() <= height);
        if height > 0 {
            assert!(rows.iter().any(|(_, selected)| *selected));
        }
    }
}

#[test]
fn narrow_palette_retains_selected_description_and_essential_keys() {
    let mut editor = InputEditor::new();
    type_text(&mut editor, "/");
    let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), 40).unwrap();
    assert_eq!(palette.title, "Commands");
    assert_eq!(palette.detail, "Find commands and keyboard shortcuts");
    assert!(crate::ui::utils::display_width(palette.keys) <= 38);
    assert!(palette.keys.contains("Enter"));
    assert!(palette.keys.contains("Esc"));
}

#[test]
fn palette_renders_above_multiline_composer_and_hides_draft_caret() {
    use crate::ui::renderer::{FakeBackend, Renderer};
    for (cols, rows) in [(80, 24), (40, 12), (20, 8)] {
        let mut renderer = Renderer::with_backend(Box::new(FakeBackend::new(cols, rows)));
        renderer.set_statusline_height(3);
        renderer.set_monochrome(true);
        renderer.sync_input_height("first\nsecond").unwrap();
        renderer
            .draw_bottom("first\nsecond", 12, &[], false)
            .unwrap();
        let before = renderer.captured_output().len();
        let mut editor = InputEditor::new();
        type_text(&mut editor, "/");
        let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), cols as usize).unwrap();
        renderer.draw_palette(&palette).unwrap();
        let output = renderer.captured_output();
        let output = &output[before..];
        let positions = regex::Regex::new(r"\x1b\[(\d+);(\d+)H").unwrap();
        let positions: Vec<_> = positions.captures_iter(output).collect();
        for position in &positions[..positions.len() - 1] {
            assert!(position[1].parse::<usize>().unwrap() <= renderer.visible_lines());
        }
        assert!(
            output.ends_with("\x1b[?25l"),
            "draft caret must be hidden while searching"
        );
    }
}

#[test]
fn composer_explains_idle_busy_and_command_states() {
    use crate::ui::renderer::{FakeBackend, Renderer};
    let mut renderer = Renderer::with_backend(Box::new(FakeBackend::new(100, 24)));
    renderer.draw_bottom("", 0, &[], false).unwrap();
    assert!(
        renderer
            .captured_output()
            .contains("Ask a question or describe a task")
    );
    renderer.draw_bottom("", 0, &[], true).unwrap();
    assert!(renderer.captured_output().contains("Enter queue request"));
    renderer.draw_bottom("/rename ", 8, &[], false).unwrap();
    assert!(renderer.captured_output().contains("/rename <name>"));
}

#[test]
fn terminal_labels_clip_wide_characters_and_strip_controls() {
    use crate::ui::utils::{clip_columns, display_width};
    assert_eq!(clip_columns("中文abc", 5), "中文a");
    assert_eq!(clip_columns("a\nb\tc", 3), "abc");
    assert_eq!(clip_columns("中", 1), "");
    for width in 0..12 {
        assert!(display_width(&clip_columns("中文a🙂é", width)) <= width);
    }
}

mod redraw {
    use super::*;
    use crate::ui::{
        feed::BlockStyle,
        renderer::{FakeBackend, Renderer},
    };

    fn renderer() -> Renderer {
        let mut renderer = Renderer::with_backend(Box::new(FakeBackend::new(80, 24)));
        for i in 0..30 {
            renderer
                .feed_mut()
                .push_line(BlockStyle::Plain, format!("chat row {i}"));
        }
        renderer
    }

    fn frame(renderer: &mut Renderer, palette: Option<&Palette>, input: &str) {
        renderer
            .synchronized_frame(|renderer| {
                renderer.sync_input_height(input)?;
                renderer.prepare_palette(palette);
                renderer.render_viewport()?;
                renderer.draw_bottom(input, input.len(), &[], false)?;
                if let Some(palette) = palette {
                    renderer.draw_palette(palette)?;
                }
                Ok(())
            })
            .unwrap();
    }

    fn command_palette() -> Palette {
        let mut editor = InputEditor::new();
        type_text(&mut editor, "/");
        Palette::from_picker(editor.picker.as_ref().unwrap(), 80).unwrap()
    }

    #[test]
    fn unchanged_palette_does_not_repaint_on_refresh() {
        let mut renderer = renderer();
        let palette = command_palette();
        frame(&mut renderer, Some(&palette), "/");
        assert!(!renderer.chat_needs_redraw());
        let before = renderer.captured_output().len();
        frame(&mut renderer, Some(&palette), "/");
        assert_eq!(
            &renderer.captured_output()[before..],
            "\x1b[?2026h\x1b[?2026l"
        );
    }

    #[test]
    fn streaming_chat_never_overwrites_visible_options() {
        let mut renderer = renderer();
        let palette = command_palette();
        frame(&mut renderer, Some(&palette), "/");
        let top = renderer.visible_lines() - palette.rows(renderer.visible_lines()).len();
        let before = renderer.captured_output().len();
        // write_line may render immediately, outside refresh_display.
        renderer
            .write_line("new streamed output", crossterm::style::Color::White)
            .unwrap();
        let output = renderer.captured_output();
        let positions = regex::Regex::new(r"\x1b\[(\d+);(\d+)H").unwrap();
        let positions: Vec<_> = positions.captures_iter(&output[before..]).collect();
        for position in &positions[..positions.len() - 1] {
            assert!(
                position[1].parse::<usize>().unwrap() <= top,
                "chat entered the menu"
            );
        }
        let before = output.len();
        frame(&mut renderer, Some(&palette), "/");
        assert_eq!(
            &renderer.captured_output()[before..],
            "\x1b[?2026h\x1b[?2026l"
        );
        frame(&mut renderer, None, "");
        assert!(renderer.captured_output().contains("new streamed output"));
    }

    #[test]
    fn filtering_shrinking_and_dismissal_restore_covered_chat() {
        let mut renderer = renderer();
        let mut palette = command_palette();
        frame(&mut renderer, Some(&palette), "/");
        let before = renderer.captured_output().len();
        palette.items.truncate(1);
        frame(&mut renderer, Some(&palette), "/help");
        let output = renderer.captured_output();
        assert!(
            output[before..].contains("chat row 24"),
            "shrinking restores newly visible chat"
        );
        let before = output.len();
        frame(&mut renderer, None, "/help");
        let output = renderer.captured_output();
        assert!(
            output[before..].contains("chat row 29"),
            "dismissal restores chat under the last menu row"
        );
        let before = output.len();
        frame(&mut renderer, None, "/help");
        assert_eq!(
            &renderer.captured_output()[before..],
            "\x1b[?2026h\x1b[?2026l"
        );
    }

    #[test]
    fn geometry_changes_and_invalidation_repaint_the_palette() {
        let mut renderer = renderer();
        let palette = command_palette();
        frame(&mut renderer, Some(&palette), "/");
        renderer.set_statusline_height(3);
        let before = renderer.captured_output().len();
        frame(&mut renderer, Some(&palette), "/\nsecond line");
        assert!(renderer.captured_output()[before..].contains("Commands"));
        renderer.resize();
        let before = renderer.captured_output().len();
        frame(&mut renderer, Some(&palette), "/\nsecond line");
        assert!(renderer.captured_output()[before..].contains("Commands"));
    }

    #[test]
    fn synchronized_frame_ends_even_when_drawing_fails() {
        let mut renderer = renderer();
        let error = renderer
            .synchronized_frame(|_| Err(std::io::Error::other("failed drawing")))
            .unwrap_err();
        assert_eq!(error.to_string(), "failed drawing");
        assert_eq!(renderer.captured_output(), "\x1b[?2026h\x1b[?2026l");
        assert!(renderer.chat_needs_redraw());
    }
}

#[cfg(feature = "subagents")]
#[test]
fn unified_model_command_keeps_each_targets_history_and_draft() {
    use crate::ui::pickers::models::{ModelChoice, ModelTarget};
    let main = ModelChoice::new("main-gateway", "main-id");
    let old_sub = ModelChoice::new("sub-gateway", "old-sub-id");
    let new_sub = ModelChoice::new("other-gateway", "new-sub-id");
    let choices = vec![main.clone(), old_sub.clone(), new_sub.clone()];
    let mut editor = InputEditor::new();
    editor.set_model_choices(choices.clone(), main.key.clone());
    editor.set_subagent_model_choices(choices.clone(), old_sub.key.clone());
    editor.load_text("preserved\n中文 draft");
    editor.set_cursor(3);
    for command in ["model", "models", "model-subagent", "models-subagent"] {
        editor.start_command_picker();
        type_text(&mut editor, command);
        press(&mut editor, KeyCode::Enter);
        let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), 80).unwrap();
        assert_eq!(
            palette.items,
            [
                "Main agent · main-id · main-gateway",
                "Subagents · old-sub-id · sub-gateway"
            ]
        );
        press(&mut editor, KeyCode::Down);
        press(&mut editor, KeyCode::Enter);
        assert!(matches!(&editor.picker, Some(Picker::Models(p))
            if p.target == ModelTarget::Subagent && p.current == old_sub.key));
        let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), 80).unwrap();
        assert_eq!(palette.title, "Models › Subagents");
        type_text(&mut editor, "other-gateway new-sub-id");
        press(&mut editor, KeyCode::Tab);
        assert!(editor.take_model_selection().is_none());
        press(&mut editor, KeyCode::Enter);
        let (target, choice) = editor.take_model_selection().unwrap();
        assert_eq!(target, ModelTarget::Subagent);
        assert_eq!(choice.key, new_sub.key);
        assert_eq!(editor.buffer, "preserved\n中文 draft");
        assert_eq!(editor.cursor, 3);
    }
    editor.set_subagent_model_choices(choices, new_sub.key.clone());
    editor.start_model_picker_for(ModelTarget::Subagent);
    assert!(matches!(&editor.picker, Some(Picker::Models(p))
        if p.current == new_sub.key && p.recent.contains(&old_sub.key)));
    editor.start_model_picker();
    press(&mut editor, KeyCode::Enter);
    assert!(matches!(&editor.picker, Some(Picker::Models(p))
        if p.current == main.key && !p.recent.contains(&old_sub.key) && !p.recent.contains(&new_sub.key)));
    assert_eq!(commands::find("/models-subagent").unwrap().name, "/model");
    assert_eq!(
        commands::search("subagent")
            .iter()
            .filter(|c| c.name == "/model")
            .count(),
        1
    );
    assert!(!commands::COMMANDS.iter().any(|c| matches!(
        c.name,
        "/model-subagent" | "/models-subagent" | "/models" | "/provider"
    )));
}

#[cfg(feature = "subagents")]
#[test]
fn subagent_selector_back_and_refresh_keep_the_target() {
    use crate::ui::pickers::models::{ModelChoice, ModelTarget};
    let choice = ModelChoice::new("gateway", "id");
    let mut editor = InputEditor::new();
    editor.set_subagent_model_choices(vec![choice.clone()], choice.key);
    type_text(&mut editor, "/model subagents ");
    assert!(matches!(&editor.picker, Some(Picker::Models(p)) if p.target == ModelTarget::Subagent));
    if let Some(Picker::Models(p)) = &mut editor.picker {
        p.busy = true;
    }
    let palette = Palette::from_picker(editor.picker.as_ref().unwrap(), 80).unwrap();
    assert_eq!(
        palette.subtitle.as_deref(),
        Some("Applies to newly started subagents")
    );
    press(&mut editor, KeyCode::Esc);
    assert!(matches!(&editor.picker, Some(Picker::ModelTarget(p)) if p.list.query == "subagents"));
    press(&mut editor, KeyCode::Esc);
    assert!(matches!(&editor.picker, Some(Picker::Command(p)) if p.query == "model"));
    press(&mut editor, KeyCode::Enter);
    type_text(&mut editor, "subagents ");
    type_text(&mut editor, "refresh");
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/model subagents refresh")
    );
    editor.start_model_picker_for(ModelTarget::Subagent);
    press(&mut editor, KeyCode::Esc);
    assert!(
        matches!(&editor.picker, Some(Picker::ModelTarget(p)) if p.selected_target() == Some(ModelTarget::Subagent))
    );
    press(&mut editor, KeyCode::Esc);
    assert!(editor.picker.is_none());
}

#[test]
fn model_commands_parse_targets_without_confusing_provider_ids() {
    use crate::ui::pickers::models::ModelTarget;
    for command in ["/model", "/models", "/provider"] {
        assert_eq!(ModelTarget::parse(command), Some((None, "")));
    }
    for (text, value) in [
        ("/model main", ""),
        ("/model main fast", "fast"),
        (" /model  main  gateway::shared-id ", "gateway::shared-id"),
        ("/model fast", "fast"),
        ("/models refresh", "refresh"),
        ("/model gateway::subagents", "gateway::subagents"),
    ] {
        assert_eq!(
            ModelTarget::parse(text),
            Some((Some(ModelTarget::Conversation), value))
        );
    }
    assert_eq!(ModelTarget::parse("/provider openai"), None);
    assert_eq!(ModelTarget::parse("/models-add"), None);
    #[cfg(feature = "subagents")]
    for (text, value) in [
        ("/model subagents", ""),
        ("/model subagents fast", "fast"),
        ("/models subagent gateway::id", "gateway::id"),
        ("/model subagents refresh", "refresh"),
        ("/model-subagent pro", "pro"),
        ("/models-subagent", ""),
    ] {
        assert_eq!(
            ModelTarget::parse(text),
            Some((Some(ModelTarget::Subagent), value))
        );
    }
}

#[cfg(feature = "subagents")]
#[test]
fn target_menu_editing_and_cancel_do_not_apply_or_leak_into_draft() {
    let mut editor = InputEditor::new();
    editor.load_text("中文 draft");
    editor.set_cursor(6);
    editor.start_model_picker();
    type_text(&mut editor, "subagent");
    press(&mut editor, KeyCode::Tab);
    assert!(matches!(&editor.picker, Some(Picker::ModelTarget(p)) if p.list.query == "subagents"));
    assert!(editor.take_model_selection().is_none());
    press(&mut editor, KeyCode::Home);
    press(&mut editor, KeyCode::Delete);
    key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    editor.handle_paste("main".into());
    press(&mut editor, KeyCode::Enter);
    type_text(&mut editor, "refresh");
    assert_eq!(
        press(&mut editor, KeyCode::Enter).as_deref(),
        Some("/model main refresh")
    );
    editor.start_model_picker();
    press(&mut editor, KeyCode::Backspace);
    assert!(editor.picker.is_none());
    assert_eq!(editor.buffer, "中文 draft");
    assert_eq!(editor.cursor, 6);
}

#[test]
fn model_add_is_absent_from_the_command_catalog_and_search() {
    for name in ["/model-add", "/models-add"] {
        assert!(commands::find(name).is_none());
        assert!(!commands::COMMANDS.iter().any(|cmd| cmd.name == name));
        assert!(!commands::search("model").iter().any(|cmd| cmd.name == name));
    }
}

#[test]
fn save_model_form_preserves_selection_draft_and_recovers_from_errors() {
    use crate::ui::pickers::models::{ModelChoice, ModelTarget};
    let choice = ModelChoice::new("openai", "test-model");
    let mut editor = InputEditor::new();
    editor.set_model_choices(vec![choice.clone()], choice.key.clone());
    editor.load_text("中文\nkeep draft");
    editor.set_cursor(6);
    editor.start_model_picker_for(ModelTarget::Conversation);
    type_text(&mut editor, "test");
    key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    assert!(matches!(&editor.picker, Some(Picker::SaveModel(p)) if p.choice.key == choice.key));
    press(&mut editor, KeyCode::Enter);
    assert!(editor.take_model_save().is_none());
    type_text(&mut editor, "fast");
    press(&mut editor, KeyCode::Enter);
    let (name, saved) = editor.take_model_save().unwrap();
    assert_eq!(name, "fast");
    assert_eq!(saved.key, choice.key);
    editor.finish_model_save(Err("Alias already exists".into()));
    assert!(
        matches!(&editor.picker, Some(Picker::SaveModel(p)) if p.input.query == "fast" && p.error.is_some())
    );
    press(&mut editor, KeyCode::Esc);
    assert!(
        matches!(&editor.picker, Some(Picker::Models(p)) if p.query == "test" && p.selected_model().unwrap().key == choice.key)
    );
    key(
        &mut editor,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
    );
    type_text(&mut editor, "fast-v2");
    press(&mut editor, KeyCode::Enter);
    editor.take_model_save().unwrap();
    editor.finish_model_save(Ok(()));
    assert!(
        editor.take_model_selection().is_none(),
        "saving must not switch models"
    );
    assert_eq!(editor.saved_model_alias("fast-v2").unwrap().key, choice.key);
    editor.set_model_choices(vec![choice.clone()], choice.key.clone());
    editor.start_model_picker_for(ModelTarget::Conversation);
    type_text(&mut editor, "fast-v2");
    assert!(
        matches!(&editor.picker, Some(Picker::Models(p)) if p.selected_model().unwrap().aliases.contains(&"fast-v2".into()))
    );
    assert_eq!(editor.buffer, "中文\nkeep draft");
    assert_eq!(editor.cursor, 6);
}
