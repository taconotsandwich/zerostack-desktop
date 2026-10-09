use super::*;

fn text(value: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(value.to_string())))
}

#[test]
fn child_args_drop_front_end_and_conversation_flags() {
    let args = [
        "--desktop",
        "--model",
        "m",
        "--session",
        "abc",
        "--name=x",
        "-c",
        "--no-session",
    ]
    .map(String::from);
    assert_eq!(
        child_args(args),
        ["--model", "m", "--no-session", "--acp"].map(String::from)
    );
}

#[test]
fn message_and_thought_chunks_become_tokens_and_reasoning() {
    assert!(matches!(
        agent_event(SessionUpdate::AgentMessageChunk(text("hi"))),
        Some(AgentEvent::Token(token)) if token == "hi"
    ));
    assert!(matches!(
        agent_event(SessionUpdate::AgentThoughtChunk(text("hmm"))),
        Some(AgentEvent::Reasoning(token)) if token == "hmm"
    ));
    assert!(agent_event(SessionUpdate::UserMessageChunk(text("me"))).is_none());
}

#[test]
fn tool_calls_and_results_keep_their_id_name_and_output() {
    let call = ToolCall::new(ToolCallId::new("c1"), "read: a.txt")
        .name("read".to_string())
        .raw_input(Some(serde_json::json!({"path": "a.txt"})));
    let Some(AgentEvent::ToolCall {
        call_id,
        name,
        args,
    }) = agent_event(SessionUpdate::ToolCall(call))
    else {
        panic!("expected a tool call");
    };
    assert_eq!((call_id.as_str(), name.as_str()), ("c1", "read"));
    assert_eq!(args["path"], "a.txt");

    let done = ToolCallUpdate::new(
        ToolCallId::new("c1"),
        ToolCallUpdateFields::new()
            .status(ToolCallStatus::Failed)
            .content(vec![ToolCallContent::from(ContentBlock::Text(
                TextContent::new("no such file".to_string()),
            ))]),
    );
    let Some(AgentEvent::ToolResult {
        call_id,
        output,
        failed,
        ..
    }) = agent_event(SessionUpdate::ToolCallUpdate(done))
    else {
        panic!("expected a tool result");
    };
    assert_eq!(
        (call_id.as_str(), output.as_str(), failed),
        ("c1", "no such file", true)
    );

    let running = ToolCallUpdate::new(
        ToolCallId::new("c1"),
        ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
    );
    assert!(agent_event(SessionUpdate::ToolCallUpdate(running)).is_none());
}

#[test]
fn a_call_without_a_name_is_named_by_its_title_and_a_done_one_is_a_subagent_call() {
    let call = ToolCall::new(ToolCallId::new("c2"), "bash: ls -la");
    assert!(matches!(
        agent_event(SessionUpdate::ToolCall(call)),
        Some(AgentEvent::ToolCall { name, .. }) if name == "bash"
    ));
    let subagent = ToolCall::new(ToolCallId::new("c3"), "[subagent] grep")
        .name("grep".to_string())
        .status(ToolCallStatus::Completed);
    assert!(matches!(
        agent_event(SessionUpdate::ToolCall(subagent)),
        Some(AgentEvent::SubagentToolCall { name, .. }) if name == "grep"
    ));
}

#[test]
fn permission_requests_name_the_tool_and_its_input() {
    let named = ToolCallUpdate::new(
        ToolCallId::new("c1"),
        ToolCallUpdateFields::new()
            .title("write: a.txt".to_string())
            .name("write".to_string())
            .raw_input(serde_json::Value::String("a.txt".to_string())),
    );
    assert_eq!(
        permission_subject(&named),
        ("write".to_string(), "a.txt".to_string())
    );
    let titled = ToolCallUpdate::new(
        ToolCallId::new("c2"),
        ToolCallUpdateFields::new().title("bash: ls".to_string()),
    );
    assert_eq!(
        permission_subject(&titled),
        ("bash".to_string(), "ls".to_string())
    );
}

#[test]
fn decisions_pick_the_option_of_their_kind() {
    let options = vec![
        PermissionOption::new("once", "Allow once", PermissionOptionKind::AllowOnce),
        PermissionOption::new("always", "Allow always", PermissionOptionKind::AllowAlways),
        PermissionOption::new("no", "Reject", PermissionOptionKind::RejectOnce),
    ];
    let chosen = |decision: Option<&UserDecision>| match outcome(decision, &options) {
        RequestPermissionOutcome::Selected(selected) => Some(selected.option_id.0.to_string()),
        _ => None,
    };
    assert_eq!(
        chosen(Some(&UserDecision::AllowOnce)).as_deref(),
        Some("once")
    );
    assert_eq!(
        chosen(Some(&UserDecision::AllowAlways("*".into()))).as_deref(),
        Some("always")
    );
    assert_eq!(chosen(Some(&UserDecision::Deny)).as_deref(), Some("no"));
    assert_eq!(chosen(None), None);
    assert!(matches!(
        outcome(Some(&UserDecision::AllowOnce), &[]),
        RequestPermissionOutcome::Cancelled
    ));
}

#[test]
fn replayed_history_is_not_streamed_but_settings_still_apply() {
    let shared = Shared::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    shared.lock().stream = Some(sender);
    shared.lock().options = vec![SessionConfigOption::select(
        "mode",
        "Mode",
        "standard",
        vec![SessionConfigSelectOption::new("standard", "standard")],
    )];
    shared.lock().replaying = true;
    shared.update(SessionUpdate::AgentMessageChunk(text("old")));
    shared.update(SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
        "yolo".to_string(),
    )));
    assert!(receiver.try_recv().is_err());
    assert!(shared.lock().text.is_empty());
    assert_eq!(select(&shared.lock().options, "mode").unwrap().0, "yolo");

    shared.lock().replaying = false;
    shared.update(SessionUpdate::AgentMessageChunk(text("new")));
    assert_eq!(shared.lock().text, "new");
    assert!(matches!(
        receiver.try_recv(),
        Ok(UiEvent::Agent(AgentEvent::Token(_)))
    ));
}

#[test]
fn options_read_select_and_boolean_values() {
    let options = vec![
        SessionConfigOption::select(
            "model",
            "Model",
            "b",
            vec![
                SessionConfigSelectOption::new("a", "A"),
                SessionConfigSelectOption::new("b", "B"),
            ],
        ),
        SessionConfigOption::boolean("reasoning", "Reasoning", true),
    ];
    assert_eq!(
        select(&options, "model"),
        Some(("b".to_string(), vec!["a".to_string(), "b".to_string()]))
    );
    assert_eq!(boolean(&options, "reasoning"), Some(true));
    assert_eq!(select(&options, "reasoning"), None);
    assert_eq!(boolean(&options, "missing"), None);
}

#[test]
fn attachments_follow_the_prompt_text() {
    let folder = std::env::temp_dir().join(format!("zerostack-blocks-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&folder).unwrap();
    let note = folder.join("note.txt");
    std::fs::write(&note, "remember").unwrap();
    let image = folder.join("dot.png");
    std::fs::write(&image, [0x89, b'P', b'N', b'G']).unwrap();
    let blocks = prompt_blocks("look", &[note, image]).unwrap();
    assert!(matches!(&blocks[0], ContentBlock::Text(text) if text.text == "look"));
    let ContentBlock::Resource(EmbeddedResource {
        resource: EmbeddedResourceResource::TextResourceContents(file),
        ..
    }) = &blocks[1]
    else {
        panic!("expected the text file");
    };
    assert_eq!(file.text, "remember");
    let ContentBlock::Resource(EmbeddedResource {
        resource: EmbeddedResourceResource::BlobResourceContents(blob),
        ..
    }) = &blocks[2]
    else {
        panic!("expected the image");
    };
    assert_eq!(blob.mime_type.as_deref(), Some("image/png"));
    assert!(prompt_blocks("x", &[folder.join("missing.txt")]).is_err());
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn notices_and_an_unsaved_session_come_from_the_zerostack_meta() {
    let mut session = crate::session::Session::new("p", "m", 0, "");
    session.add_message(crate::session::MessageRole::User, "hello");
    let meta = |value: serde_json::Value| -> Meta {
        serde_json::from_value(serde_json::json!({ "zerostack": value })).unwrap()
    };
    let notices = meta(serde_json::json!({ "notices": ["docs did not connect"] }));
    assert_eq!(meta_notices(Some(&notices)), ["docs did not connect"]);
    assert!(meta_notices(None).is_empty());
    assert!(meta_session(Some(&notices)).is_none());

    let carried = meta(serde_json::json!({ "session": session }));
    let read = meta_session(Some(&carried)).unwrap();
    assert_eq!(read.id, session.id);
    assert_eq!(read.messages[0].content, "hello");
    assert!(meta_notices(Some(&carried)).is_empty());
}

#[cfg(feature = "mcp")]
#[test]
fn login_url_comes_from_the_mcp_login_announcement() {
    let line = "open this URL to authorize 'docs':\nhttps://auth.example.com/x?y=1\nwaiting on 127.0.0.1:4000 ...\n";
    assert_eq!(
        login_url(line).as_deref(),
        Some("https://auth.example.com/x?y=1")
    );
    assert_eq!(login_url("open this file"), None);
    assert_eq!(login_url("open this URL to authorize 'docs':\n"), None);
}

#[tokio::test]
async fn a_process_that_exits_fails_the_open_instead_of_hanging() {
    let opened = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Conversation::open(
            Path::new("/usr/bin/false"),
            &[],
            &std::env::temp_dir(),
            None,
        ),
    )
    .await
    .expect("open returned");
    assert!(opened.is_err());
}
