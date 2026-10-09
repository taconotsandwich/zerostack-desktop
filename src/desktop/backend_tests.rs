use super::*;
use crate::session::MessageRole;

#[test]
fn conversation_operations_become_prompt_text() {
    let session = Session::new("p", "m", 0, "");
    let text = |operation: Operation| prompt_text(&operation, &session);
    assert_eq!(text(Operation::Prompt("hi".into())).as_deref(), Some("hi"));
    assert_eq!(text(Operation::Undo).as_deref(), Some("/undo"));
    assert_eq!(
        text(Operation::CompressConversation { instructions: None }).as_deref(),
        Some("/compress")
    );
    assert_eq!(
        text(Operation::CompressConversation {
            instructions: Some("keep code".into())
        })
        .as_deref(),
        Some("/compress keep code")
    );
    assert_eq!(
        text(Operation::RunShell {
            command: "ls".into()
        })
        .as_deref(),
        Some("!ls")
    );
    assert_eq!(
        text(Operation::AskSeparateQuestion {
            question: "why".into()
        })
        .as_deref(),
        Some("/btw why")
    );
    assert_eq!(text(Operation::NewSession), None);
    assert_eq!(text(Operation::ClearContextFiles), None);
}

#[test]
fn rewind_names_the_user_message_by_its_position() {
    let mut session = Session::new("p", "m", 0, "");
    session.add_message(MessageRole::User, "one");
    session.add_message(MessageRole::Assistant, "ok");
    session.add_message(MessageRole::User, "two");
    assert_eq!(
        prompt_text(&Operation::Rewind(2), &session).as_deref(),
        Some("/rewind 2")
    );
    assert_eq!(prompt_text(&Operation::Rewind(1), &session), None);
}

#[test]
fn run_output_echoes_the_input_and_keeps_the_error() {
    let output = run_output("/model x", Ok(("switched".into(), false)));
    assert_eq!(output.text, "> /model x\nswitched");
    assert!(matches!(output.kind, RunKind::Command));
    assert!(output.error.is_none());

    let output = run_output("hello\nthere", Err("rate limited".into()));
    assert_eq!(output.text, "> hello\n> there\nerror: rate limited");
    assert!(matches!(output.kind, RunKind::Agent));
    assert_eq!(output.error.as_deref(), Some("rate limited"));

    assert!(run_output("go", Ok((String::new(), true))).cancelled);
}

#[test]
fn attaching_keeps_existing_files_once_and_reports_missing_ones() {
    let file = std::env::temp_dir().join(format!("zerostack-attach-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&file, "x").unwrap();
    let missing = file.with_extension("missing");
    let mut files = Vec::new();
    let errors = attach(
        &mut files,
        vec![file.clone(), file.clone(), missing.clone()],
    );
    assert_eq!(files, vec![file.clone()]);
    assert_eq!(
        errors,
        Some(format!("error: no such file: {}", missing.display()))
    );
    assert_eq!(attach(&mut files, vec![file.clone()]), None);
    std::fs::remove_file(&file).unwrap();
}

#[test]
fn read_doc_rejects_empty_and_path_names() {
    // Reading a real doc goes through the global data directory, which other
    // tests repoint concurrently, so only the name checks are tested here.
    assert!(read_doc("").is_err());
    assert!(read_doc("../secrets.md").is_err());
    assert!(read_doc("a/b.md").is_err());
    assert!(read_doc(".hidden").is_err());
}
