use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::worker::{self, Worker};
use crate::cli::Cli;

#[test]
fn project_switch_restarts_worker_after_process_initialization() {
    const CHILD: &str = "ZS_DESKTOP_STARTUP_TEST";
    if let Some(root) = std::env::var_os(CHILD) {
        let cli = Cli {
            desktop: true,
            api_key: Some("test-key".into()),
            no_context_files: true,
            #[cfg(feature = "hooks")]
            no_hooks: true,
            ..Default::default()
        };
        crate::logging::install_panic_hook();
        crate::logging::init(&cli);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for project in ["first", "second", "first"] {
                let directory = Path::new(&root).join(project).canonicalize().unwrap();
                let (worker, ready) =
                    Worker::start(cli.clone(), Some(directory.display().to_string()));
                let snapshot =
                    tokio::time::timeout(Duration::from_secs(10), worker::receive(ready))
                        .await
                        .expect("project startup timed out")
                        .expect("project startup failed");
                assert_eq!(Path::new(snapshot.session.working_dir.as_str()), directory);
                assert!(snapshot.session.messages.is_empty());
                tokio::time::timeout(Duration::from_secs(10), worker.stop())
                    .await
                    .expect("previous worker did not stop");
            }
        });
        return;
    }

    // Project switching changes the process directory and initializes global
    // logging. Exercise it in a child so parallel tests keep their own state.
    let root = std::env::temp_dir().join(format!("zerostack-startup-{}", uuid::Uuid::new_v4()));
    for directory in ["first", "second", "data", "config"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    std::fs::write(
        root.join("config/config.toml"),
        r#"provider = "anthropic"
model = "claude-sonnet-4-5"
auto-update-prompts = false
auto-update-themes = false
enable-exa-mcp = false
enable-context7-mcp = false
enable-grepapp-mcp = false
sandbox = false
"#,
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop::startup_tests::project_switch_restarts_worker_after_process_initialization",
            "--nocapture",
        ])
        .env(CHILD, &root)
        .env("ZS_DATA_DIR", root.join("data"))
        .env("ZS_CONFIG_DIR", root.join("config"))
        .current_dir(&root)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
