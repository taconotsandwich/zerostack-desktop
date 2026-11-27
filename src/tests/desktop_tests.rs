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

#[test]
fn icons_are_24px_svgs() {
    use crate::extras::desktop::icon::{Icon, LOGO};
    for &icon in Icon::ALL {
        let svg = std::str::from_utf8(icon.bytes()).expect("icon is UTF-8");
        assert!(svg.starts_with("<svg"), "{icon:?} is not an SVG");
        assert!(
            svg.contains(r#"viewBox="0 0 24 24""#),
            "{icon:?} is not 24px"
        );
    }
    assert!(std::str::from_utf8(LOGO).unwrap().starts_with("<svg"));
}

#[test]
fn fonts_are_opentype() {
    for font in crate::extras::desktop::theme::FONT_FILES {
        assert_eq!(&font[..4], b"OTTO", "font is not CFF OpenType");
    }
}
