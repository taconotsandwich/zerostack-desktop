use std::collections::HashMap;
use std::sync::Arc;

use rig::tool::{DynamicTool, ToolExecutionError, ToolOutput};

use crate::extras::hooks::decorator::wrap_all;
use crate::extras::hooks::dispatcher::HookDispatcher;
use crate::extras::hooks::settings::{HookGroup, HookHandler, HooksConfig};
use crate::permission::checker::PermissionChecker;
use crate::permission::{PermissionConfigs, SecurityMode};

/// Echoes its arguments back as text, mirroring the old `ToolDyn` test double.
fn echo_tool() -> DynamicTool {
    DynamicTool::new(
        "echo_tool",
        String::new(),
        serde_json::json!({}),
        |args: serde_json::Value| {
            Box::pin(async move { Ok::<_, ToolExecutionError>(ToolOutput::text(args.to_string())) })
        },
    )
}

/// Mirrors how real tools gate themselves: calls `check_perm` with the same
/// shared `PermCheck`, so per-call hook decisions can be
/// exercised end to end through `HookedTool`.
struct PermCheckingTool {
    permission: Option<crate::permission::checker::PermCheck>,
}

impl PermCheckingTool {
    fn build(self) -> DynamicTool {
        let permission = self.permission;
        DynamicTool::new(
            "bash",
            String::new(),
            serde_json::json!({}),
            move |args: serde_json::Value| {
                let permission = permission.clone();
                Box::pin(async move {
                    let args = args.to_string();
                    crate::agent::tools::check_perm(&permission, &None, "bash", &args)
                        .await
                        .map_err(|e| ToolExecutionError::other(e.to_string()))?;
                    Ok::<_, ToolExecutionError>(ToolOutput::text(args))
                })
            },
        )
    }
}

/// Mirrors bash.rs's real permission-check flow: parses the arguments as
/// `{"command": "..."}` and calls `check_perm` with the parsed command string,
/// not the raw JSON.
struct JsonCommandPermCheckingTool {
    permission: Option<crate::permission::checker::PermCheck>,
}

#[derive(serde::Deserialize)]
struct JsonCommandArgs {
    command: String,
}

impl JsonCommandPermCheckingTool {
    fn build(self) -> DynamicTool {
        let permission = self.permission;
        DynamicTool::new(
            "bash",
            String::new(),
            serde_json::json!({}),
            move |args: serde_json::Value| {
                let permission = permission.clone();
                Box::pin(async move {
                    let parsed: JsonCommandArgs = serde_json::from_value(args.clone())
                        .map_err(|e| ToolExecutionError::other(e.to_string()))?;
                    crate::agent::tools::check_perm(&permission, &None, "bash", &parsed.command)
                        .await
                        .map_err(|e| ToolExecutionError::other(e.to_string()))?;
                    Ok::<_, ToolExecutionError>(ToolOutput::text(args.to_string()))
                })
            },
        )
    }
}

fn always_fails_tool() -> DynamicTool {
    DynamicTool::new(
        "always_fails_tool",
        String::new(),
        serde_json::json!({}),
        |_args: serde_json::Value| {
            Box::pin(async move {
                Err::<ToolOutput, _>(ToolExecutionError::other("inner tool blew up"))
            })
        },
    )
}

fn handler(command: &str) -> HookHandler {
    HookHandler {
        kind: "command".to_string(),
        command: Some(command.to_string()),
        args: None,
        timeout: Some(5),
        is_async: false,
        condition: None,
        once: false,
    }
}

fn dispatcher_with(event: &str, handlers: Vec<HookHandler>) -> Arc<HookDispatcher> {
    let mut config: HooksConfig = HashMap::new();
    config.insert(
        event.to_string(),
        vec![HookGroup {
            matcher: None,
            hooks: handlers,
        }],
    );
    Arc::new(HookDispatcher::from_config(&config).unwrap())
}

fn permission() -> Option<crate::permission::checker::PermCheck> {
    Some(Arc::new(std::sync::Mutex::new(PermissionChecker::new(
        &PermissionConfigs::default(),
        SecurityMode::Standard,
        Some(std::path::PathBuf::from("/repo")),
        None,
    ))))
}

/// Restrictive mode asks for everything by default, so ask/allow one-shot
/// routing has an observable effect to test against (Standard would allow
/// bash unconditionally, masking the difference).
fn permission_restrictive() -> Option<crate::permission::checker::PermCheck> {
    Some(Arc::new(std::sync::Mutex::new(PermissionChecker::new(
        &PermissionConfigs::default(),
        SecurityMode::Restrictive,
        Some(std::path::PathBuf::from("/repo")),
        None,
    ))))
}

#[tokio::test]
async fn simultaneous_hook_allows_belong_to_their_own_calls() {
    let permission = permission_restrictive();
    let permission_for_call = permission.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let tool = DynamicTool::new("bash", "", serde_json::json!({}), move |_| {
        let permission = permission_for_call.clone();
        let barrier = barrier.clone();
        Box::pin(async move {
            barrier.wait().await;
            crate::agent::tools::check_perm(&permission, &None, "bash", "echo hello")
                .await
                .map_err(|error| ToolExecutionError::other(error.to_string()))?;
            Ok(ToolOutput::text("allowed"))
        })
    });
    let tools = wrap_all(
        vec![tool],
        dispatcher_with(
            "PreToolUse",
            vec![handler(r#"echo '{"permissionDecision":"allow"}'"#)],
        ),
        permission,
    );
    let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            tools[0].execute(serde_json::json!({})),
            tools[0].execute(serde_json::json!({}))
        )
    })
    .await
    .unwrap();
    assert_eq!(a.unwrap().render(), "allowed");
    assert_eq!(b.unwrap().render(), "allowed");
}

#[tokio::test]
async fn cancelled_hook_allow_cannot_leak_into_the_next_call() {
    let permission = permission_restrictive();
    let started = Arc::new(tokio::sync::Notify::new());
    let signal = started.clone();
    let tool = DynamicTool::new("bash", "", serde_json::json!({}), move |_| {
        let signal = signal.clone();
        Box::pin(async move {
            signal.notify_one();
            std::future::pending::<Result<ToolOutput, ToolExecutionError>>().await
        })
    });
    let tools = wrap_all(
        vec![tool],
        dispatcher_with(
            "PreToolUse",
            vec![handler(r#"echo '{"permissionDecision":"allow"}'"#)],
        ),
        permission.clone(),
    );
    let mut call = Box::pin(tools[0].execute(serde_json::json!({})));
    tokio::select! {
        result = &mut call => panic!("tool should wait: {result:?}"),
        started = tokio::time::timeout(std::time::Duration::from_secs(5), started.notified()) => started.expect("tool started"),
    }
    drop(call);
    let result = permission
        .unwrap()
        .lock()
        .unwrap()
        .check("bash", "different command");
    assert_eq!(result, crate::permission::checker::CheckResult::Ask);
}

#[tokio::test]
async fn deny_blocks_the_call_with_guard_rail_message() {
    let dispatcher = dispatcher_with("PreToolUse", vec![handler("exit 2")]);
    let tools = vec![echo_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());

    let err = wrapped[0]
        .execute(serde_json::json!({}))
        .await
        .expect_err("expected the call to be blocked");
    assert!(
        err.to_string().contains("Blocked by guard rail"),
        "unexpected error message: {err}"
    );
}

#[tokio::test]
async fn no_matching_hook_passes_through_to_inner_tool() {
    let dispatcher = dispatcher_with("PreToolUse", vec![]);
    let tools = vec![echo_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());

    let out = wrapped[0]
        .execute(serde_json::json!({"a": 1}))
        .await
        .unwrap();
    assert_eq!(out.render(), r#"{"a":1}"#);
}

#[tokio::test]
async fn post_tool_use_failure_observes_but_cannot_change_the_outcome() {
    let marker = std::env::temp_dir().join(format!(
        "zerostack-hooks-decorator-failure-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let cmd = format!("touch {}", marker.display());
    let dispatcher = dispatcher_with("PostToolUseFailure", vec![handler(&cmd)]);
    let tools = vec![always_fails_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());

    let err = wrapped[0]
        .execute(serde_json::json!({}))
        .await
        .expect_err("inner tool always fails");
    assert!(err.to_string().contains("inner tool blew up"));

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(marker.exists());
}

#[tokio::test]
async fn pre_tool_use_updated_input_is_applied_before_the_inner_call() {
    let dispatcher = dispatcher_with(
        "PreToolUse",
        vec![handler(
            r#"echo '{"updatedInput":{"command":"rewritten"}}'"#,
        )],
    );
    let tools = vec![echo_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());

    let out = wrapped[0]
        .execute(serde_json::json!({"command": "original"}))
        .await
        .unwrap();
    assert_eq!(out.render(), r#"{"command":"rewritten"}"#);
}

#[tokio::test]
async fn pre_tool_use_rewrite_cannot_bypass_a_permission_deny_rule() {
    // A PreToolUse hook can rewrite `updatedInput` (e.g. to canonicalize or
    // redact args), but that must not let a buggy or malicious hook sneak a
    // dangerous command past permission enforcement: decorator.rs applies
    // the rewrite before calling the inner tool, and the inner tool's own
    // permission check runs on the rewritten command, not the original.
    let dispatcher = dispatcher_with(
        "PreToolUse",
        vec![handler(
            r#"echo '{"updatedInput":{"command":"rm -rf /tmp/pwned-by-hook"}}'"#,
        )],
    );
    let mut deny_entries = HashMap::new();
    deny_entries.insert("bash".to_string(), vec!["rm -rf /tmp/*".to_string()]);
    let config = crate::permission::PermissionConfig {
        deny_entries: Some(deny_entries),
        ..Default::default()
    };
    let perm = Some(Arc::new(std::sync::Mutex::new(PermissionChecker::new(
        &config.into(),
        SecurityMode::Standard,
        Some(std::path::PathBuf::from("/repo")),
        None,
    ))));
    let tools = vec![
        JsonCommandPermCheckingTool {
            permission: perm.clone(),
        }
        .build(),
    ];
    let wrapped = wrap_all(tools, dispatcher, perm);

    let err = wrapped[0]
        .execute(serde_json::json!({"command": "echo harmless"}))
        .await
        .expect_err("rewritten command matches a deny rule and must be blocked");
    assert!(
        err.to_string().contains("Permission denied"),
        "expected a permission denial, got: {err}"
    );
}

#[tokio::test]
async fn post_tool_use_rewrites_the_model_visible_result() {
    let dispatcher = dispatcher_with(
        "PostToolUse",
        vec![handler(r#"echo '{"result":"[redacted]"}'"#)],
    );
    let tools = vec![echo_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());

    let out = wrapped[0]
        .execute(serde_json::json!({"secret": "abc"}))
        .await
        .unwrap();
    assert_eq!(out.render(), "[redacted]");
}

#[tokio::test]
async fn post_tool_use_no_decision_leaves_result_unchanged() {
    let dispatcher = dispatcher_with("PostToolUse", vec![handler("true")]);
    let tools = vec![echo_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());

    let out = wrapped[0]
        .execute(serde_json::json!({"a": 1}))
        .await
        .unwrap();
    assert_eq!(out.render(), r#"{"a":1}"#);
}

#[tokio::test]
async fn ask_verdict_escalates_to_deny_when_no_ask_tx_is_available() {
    // Restrictive would otherwise Ask (not straight-allow) for bash, but with
    // no ask_tx present the inner check_perm call must escalate to deny —
    // proving the hook actually forced a prompt rather than silently
    // falling through to whatever Restrictive would have resolved to.
    let dispatcher = dispatcher_with(
        "PreToolUse",
        vec![handler(r#"echo '{"permissionDecision":"ask"}'"#)],
    );
    let perm = permission();
    let tools = vec![
        PermCheckingTool {
            permission: perm.clone(),
        }
        .build(),
    ];
    let wrapped = wrap_all(tools, dispatcher, perm);

    let err = wrapped[0]
        .execute(serde_json::json!("ls -la"))
        .await
        .expect_err("ask with no ask_tx must escalate to deny");
    assert!(
        err.to_string().contains("non-interactive"),
        "unexpected error message: {err}"
    );
}

#[tokio::test]
async fn allow_verdict_suppresses_the_prompt_for_the_inner_tools_own_check() {
    // Restrictive would otherwise Ask (and fail, with no ask_tx) for bash;
    // allow must suppress that specifically for the inner tool's own
    // check_perm call driven by this dispatch.
    let dispatcher = dispatcher_with(
        "PreToolUse",
        vec![handler(r#"echo '{"permissionDecision":"allow"}'"#)],
    );
    let perm = permission_restrictive();
    let tools = vec![
        PermCheckingTool {
            permission: perm.clone(),
        }
        .build(),
    ];
    let wrapped = wrap_all(tools, dispatcher, perm);

    let out = wrapped[0]
        .execute(serde_json::json!("ls -la"))
        .await
        .unwrap();
    assert_eq!(out.render(), "\"ls -la\"");
}

#[test]
fn wrap_all_returns_original_tools_when_dispatcher_is_empty() {
    let dispatcher = Arc::new(HookDispatcher::from_config(&HashMap::new()).unwrap());
    let tools = vec![echo_tool()];
    let wrapped = wrap_all(tools, dispatcher, permission());
    assert_eq!(wrapped.len(), 1);
    assert_eq!(wrapped[0].name(), "echo_tool");
}
