use std::path::PathBuf;

use crate::cli::Cli;
use crate::logging;

use clap::Parser;

fn parse_cli(args: &[&str]) -> Cli {
    let mut full = vec!["zerostack"];
    full.extend(args);
    Cli::parse_from(full)
}

#[test]
fn test_resolve_log_path_default_no_verbose() {
    let cli = parse_cli(&[]);
    let log_path = logging::resolve_log_path(&cli);
    assert!(log_path.is_none());
}

#[test]
fn test_resolve_log_path_verbose() {
    let cli = parse_cli(&["-v"]);
    let log_path = logging::resolve_log_path(&cli);
    assert!(log_path.is_some());
    let path = log_path.unwrap();
    assert!(path.to_string_lossy().contains("zerostack-"));
    assert!(path.to_string_lossy().ends_with(".log"));
    assert!(
        path.to_string_lossy()
            .contains(&std::process::id().to_string())
    );
}

#[test]
fn test_resolve_log_path_cli_override() {
    let cli = parse_cli(&["--log-file", "/tmp/test-zerostack.log"]);
    let log_path = logging::resolve_log_path(&cli);
    assert_eq!(log_path, Some(PathBuf::from("/tmp/test-zerostack.log")));
}

#[test]
fn test_resolve_log_path_cli_overrides_verbose() {
    let cli = parse_cli(&["-v", "--log-file", "/tmp/override.log"]);
    let log_path = logging::resolve_log_path(&cli);
    assert_eq!(log_path, Some(PathBuf::from("/tmp/override.log")));
}

#[cfg(feature = "logging")]
#[test]
fn test_build_stderr_filter_default() {
    let cli = parse_cli(&[]);
    let filter = logging::build_stderr_filter(&cli);
    let s = format!("{}", filter);
    assert!(s.contains("warn"));
}

#[cfg(feature = "logging")]
#[test]
fn test_build_stderr_filter_log_level() {
    let cli = parse_cli(&["--log-level", "info"]);
    let filter = logging::build_stderr_filter(&cli);
    let s = format!("{}", filter);
    assert!(s.contains("info"));
}

#[cfg(feature = "logging")]
#[test]
fn test_build_stderr_filter_invalid_log_level_does_not_panic() {
    let cli = parse_cli(&["--log-level", "invalid"]);
    let _filter = logging::build_stderr_filter(&cli);
}

#[cfg(feature = "logging")]
#[test]
fn file_logging_retains_mcp_cleanup_failures_for_diagnosis() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let captured = Capture(Arc::default());
    let writer = captured.clone();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .with_filter(logging::build_file_filter()),
    );
    tracing::subscriber::with_default(subscriber, || {
        tracing::error!(target: "rmcp::transport::streamable_http_client", "fail to delete session: diagnostic fixture");
        tracing::trace!(target: "rmcp::transport", "unrequested protocol trace");
    });
    let text = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(text.contains("fail to delete session: diagnostic fixture"));
    assert!(!text.contains("unrequested protocol trace"));
}

#[test]
fn test_verbose_flag_is_false_by_default() {
    let cli = parse_cli(&[]);
    assert!(!cli.verbose);
}

#[test]
fn test_verbose_flag_set() {
    let cli = parse_cli(&["-v"]);
    assert!(cli.verbose);
}

#[test]
fn test_verbose_flag_long_form() {
    let cli = parse_cli(&["--verbose"]);
    assert!(cli.verbose);
}

#[test]
fn test_crash_log_path_format() {
    let path = logging::resolve_crash_log_path();
    let s = path.to_string_lossy();
    assert!(s.contains("crashes"));
    assert!(s.contains("zerostack-crash-"));
    assert!(s.ends_with(".log"));
    assert!(s.contains(&std::process::id().to_string()));
}

#[test]
fn test_crash_log_dir_is_under_data_logs() {
    let dir = logging::crash_log_dir();
    let s = dir.to_string_lossy();
    assert!(s.contains("logs"));
    assert!(s.ends_with("crashes"));
}
