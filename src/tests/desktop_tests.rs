//! `--desktop` entry point: flag parsing and the window title.

use clap::Parser;

use crate::cli::Cli;
use crate::extras::desktop::app::folder_name;

fn cli_with(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("zerostack").chain(args.iter().copied())).unwrap()
}

#[test]
fn desktop_flag_parses() {
    assert!(cli_with(&["--desktop"]).desktop);
    assert!(!cli_with(&[]).desktop);
}

#[test]
fn folder_name_is_last_component() {
    assert_eq!(folder_name("/Users/me/Projects/zerostack"), "zerostack");
    assert_eq!(folder_name("/Users/me/Projects/zerostack/"), "zerostack");
    assert_eq!(folder_name("/"), "/");
}
