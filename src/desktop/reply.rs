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

/// One piece of a sent message: a passage it replied to, or its own words.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Part {
    Quote(String),
    Text(String),
}

/// Read a sent message back as the quotes it replied to and its own text,
/// without the reply markers. A blockquote with no marker is the author's
/// own text and stays as written.
pub(super) fn parts(message: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text: Vec<&str> = Vec::new();
    let mut quote: Option<Vec<&str>> = None;
    for line in message.lines() {
        if let Some(lines) = &mut quote {
            if let Some(rest) = line.strip_prefix('>') {
                lines.push(rest.strip_prefix(' ').unwrap_or(rest));
                continue;
            }
            push_quote(&mut parts, lines);
            quote = None;
        }
        if line.starts_with(MARKER) && line.trim_end().ends_with("-->") {
            push_text(&mut parts, &mut text);
            quote = Some(Vec::new());
        } else {
            text.push(line);
        }
    }
    if let Some(lines) = &quote {
        push_quote(&mut parts, lines);
    }
    push_text(&mut parts, &mut text);
    parts
}

fn push_quote(parts: &mut Vec<Part>, lines: &[&str]) {
    if !lines.is_empty() {
        parts.push(Part::Quote(lines.join("\n")));
    }
}

fn push_text(parts: &mut Vec<Part>, lines: &mut Vec<&str>) {
    let joined = lines.join("\n");
    let text = joined.trim_end().trim_start_matches(['\n', '\r']);
    if !text.is_empty() {
        parts.push(Part::Text(text.to_string()));
    }
    lines.clear();
}

#[cfg(test)]
mod tests {
    use super::{Part, parts, quote};

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

    #[test]
    fn replies_read_back_as_quotes_and_text_without_markers() {
        let message = quote("", "6 passed.\n\n- Skipped: totals");
        let message = format!("{message}Add the totals line.\n");
        let message = quote(&message, "Second");
        let message = format!("{message}And this.");
        assert_eq!(
            parts(&message),
            [
                Part::Quote("6 passed.\n\n- Skipped: totals".into()),
                Part::Text("Add the totals line.".into()),
                Part::Quote("Second".into()),
                Part::Text("And this.".into()),
            ]
        );
    }

    #[test]
    fn plain_messages_and_unmarked_blockquotes_stay_text() {
        assert_eq!(
            parts("> my own quote\n\nreply"),
            [Part::Text("> my own quote\n\nreply".into())]
        );
        assert_eq!(parts("hi"), [Part::Text("hi".into())]);
        assert_eq!(
            parts("<!-- reply -->\nno quote"),
            [Part::Text("no quote".into())]
        );
    }
}
