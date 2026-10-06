//! Quoted replies: an earlier message lands in the composer as a
//! `<!-- reply N -->` marker plus a markdown blockquote, the format Claude
//! Desktop sends, so models and exported sessions read it as a quote.

const MARKER: &str = "<!-- reply";

/// Append `excerpt` to `draft` as the next quote, leaving the cursor on a
/// fresh line below it for the response.
pub(super) fn quote(draft: &str, excerpt: &str) -> String {
    let number = draft
        .lines()
        .filter(|line| line.starts_with(MARKER))
        .count()
        + 1;
    let marker = match number {
        1 => format!("{MARKER} -->"),
        _ => format!("{MARKER} {number} -->"),
    };
    let quoted = excerpt
        .trim_end()
        .trim_start_matches(['\n', '\r'])
        .lines()
        .map(|line| match line.trim_end() {
            "" => ">".to_string(),
            line => format!("> {line}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let draft = draft.trim_end();
    let separator = if draft.is_empty() { "" } else { "\n\n" };
    format!("{draft}{separator}{marker}\n{quoted}\n\n")
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn first_quote_is_unnumbered_and_keeps_blank_lines_inside_the_quote() {
        assert_eq!(
            quote("", "\nFirst line\n\n    indented\n"),
            "<!-- reply -->\n> First line\n>\n>     indented\n\n"
        );
    }

    #[test]
    fn later_quotes_are_numbered_and_follow_the_existing_draft() {
        let draft = quote("", "One");
        let draft = format!("{draft}My answer\n");
        assert_eq!(
            quote(&draft, "Two"),
            "<!-- reply -->\n> One\n\nMy answer\n\n<!-- reply 2 -->\n> Two\n\n"
        );
    }
}
