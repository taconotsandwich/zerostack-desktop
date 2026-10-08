//! An edit call's replacements as old/new text, for showing them to a user.

use super::edit::{parse_blocks, parse_tagged_line};
use super::{EditArgs, EditBlock};

/// The replacements an edit call asks for, as old/new text pairs, for
/// showing the edit to a user. Lines of a hashedit call lose their
/// `N|TAG ` prefix. A call that does not parse yields no pairs.
pub(crate) fn edit_hunks(args: &EditArgs) -> Vec<EditBlock> {
    if let Some(block) = &args.block {
        return parse_blocks(block).unwrap_or_default();
    }
    let untag = |raw: &str| {
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let stripped = line.trim_start_matches([' ', '\t']);
                match parse_tagged_line(line) {
                    Some(_) => stripped.split_once(' ').map_or("", |(_, rest)| rest),
                    None => stripped,
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    args.edits
        .iter()
        .flatten()
        .map(|op| EditBlock {
            search: untag(op.lines.as_deref().or(op.line.as_deref()).unwrap_or("")),
            replace: op.text.clone(),
        })
        .collect()
}
