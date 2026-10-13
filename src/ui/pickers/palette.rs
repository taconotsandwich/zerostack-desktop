//! Layout for command and argument suggestions. Painting goes through the
//! renderer so suggestions respect composer height and can be tested headless.

use crate::ui::{commands, input::Picker};

pub(crate) struct Palette {
    pub title: String,
    pub subtitle: Option<String>,
    pub query: Option<String>,
    pub items: Vec<String>,
    pub selected: usize,
    pub detail: String,
    pub empty: &'static str,
    pub keys: &'static str,
}

impl Palette {
    pub fn from_picker(picker: &Picker, width: usize) -> Option<Self> {
        if !picker.active() {
            return None;
        }
        match picker {
            Picker::Command(p) => {
                let command = p.selected_name().and_then(commands::find);
                Some(Self {
                    title: if width < 54 {
                        "Commands"
                    } else {
                        "Commands · search names or descriptions"
                    }
                    .into(),
                    subtitle: None,
                    query: Some(query_line("Search", &p.query, p.cursor, width)),
                    items: p
                        .matches
                        .iter()
                        .map(|name| {
                            let cmd = commands::find(name).expect("catalog command");
                            if width >= 54 {
                                format!("{:<19} {}", cmd.name, cmd.description)
                            } else {
                                cmd.name.to_string()
                            }
                        })
                        .collect(),
                    selected: p.selected,
                    detail: command
                        .map(|cmd| {
                            if width < 54 {
                                cmd.description.to_string()
                            } else {
                                format!("{} · {}", cmd.group, cmd.usage())
                            }
                        })
                        .unwrap_or_default(),
                    empty: "No matching commands. Backspace to broaden your search.",
                    keys: if width < 54 {
                        "↑↓ · Tab fill · Enter select · Esc"
                    } else {
                        "↑↓ navigate · Tab complete · Enter select · Esc close"
                    },
                })
            }
            Picker::Prefixed(p, prefix) | Picker::Argument(p, prefix) => {
                let cmd = commands::find(prefix.trim());
                Some(Self {
                    title: if width < 54 {
                        format!("{} · choose a value", prefix.trim())
                    } else {
                        format!("{} · choose a value or type your own", prefix.trim())
                    },
                    subtitle: None,
                    query: if matches!(picker, Picker::Argument(..)) {
                        Some(query_line("Value", &p.query, p.cursor, width))
                    } else {
                        None
                    },
                    items: p
                        .matches
                        .iter()
                        .map(|item| {
                            let hint = match (prefix.trim(), item.as_str()) {
                                ("/mode", "standard") => "Project access; ask for external paths",
                                ("/mode", "restrictive") => "Ask before every operation",
                                ("/mode", "readonly") => "Read only; no changes or shell commands",
                                ("/mode", "guarded") => "Read freely; ask before changes",
                                ("/mode", "yolo") => {
                                    "Allow actions; ask for destructive shell commands"
                                }
                                ("/queue", "ls") => "Show queued requests",
                                ("/queue", "pop") => "Remove the last queued request",
                                ("/queue", "clear") => "Remove all queued requests",
                                ("/prompt" | "/theme", "default") => "Reset to default",
                                _ => "",
                            };
                            if width >= 54 && !hint.is_empty() {
                                format!("{item:<15} {hint}")
                            } else {
                                item.clone()
                            }
                        })
                        .collect(),
                    selected: p.selected,
                    detail: cmd
                        .map(|cmd| cmd.usage())
                        .unwrap_or_else(|| "Choose a prompt for your next request".into()),
                    empty: "No suggestions. Keep typing; Enter uses your value.",
                    keys: if width < 54 {
                        "↑↓ · Tab fill · Enter select · Esc"
                    } else if *prefix == "." {
                        "↑↓ navigate · Tab/Enter complete · Esc close"
                    } else {
                        "↑↓ navigate · Tab complete · Enter apply · Esc back"
                    },
                })
            }
            Picker::ModelTarget(p) => Some(Self {
                title: "Models · choose an agent".into(),
                subtitle: None,
                query: Some(query_line("Search", &p.list.query, p.list.cursor, width)),
                items: p.list.matches.iter().map(|name| p.row(name)).collect(),
                selected: p.list.selected,
                detail: p
                    .selected_target()
                    .map(|target| target.applies_to().into())
                    .unwrap_or_default(),
                empty: "Choose main or subagents. Backspace to broaden your search.",
                keys: if width < 54 {
                    "↑↓ · Enter select · Esc back"
                } else {
                    "↑↓ navigate · Tab complete · Enter select · Esc back"
                },
            }),
            Picker::SaveModel(p) => Some(Self {
                title: "Save model as…".into(),
                subtitle: Some(p.choice.detail()),
                query: Some(query_line("Name", &p.input.query, p.input.cursor, width)),
                items: vec![if p.input.query.trim().is_empty() {
                    "Enter a short name, e.g. fast".into()
                } else {
                    format!("Save as {}", p.input.query.trim())
                }],
                selected: 0,
                detail: p.error.clone().unwrap_or_else(|| {
                    format!("Save in {}", crate::config::config_file_path().display())
                }),
                empty: "",
                keys: "Enter save · Esc back",
            }),
            Picker::Models(p) => Some(Self {
                title: p.target.title().into(),
                subtitle: Some(if let Some(notice) = &p.notice {
                    notice.clone()
                } else if p.busy {
                    p.target.applies_to().into()
                } else {
                    format!("Current: {} · {}", p.current.id, p.current.provider)
                }),
                query: Some(query_line("Search", &p.query, p.cursor, width)),
                items: p.matches.iter().map(|choice| p.row(choice)).collect(),
                selected: p.selected,
                detail: p
                    .selected_model()
                    .map(|choice| choice.detail())
                    .unwrap_or_else(|| {
                        "Type refresh to reload the current provider's models".into()
                    }),
                empty: "No matching models. Enter a model ID or type refresh.",
                keys: if width < 54 {
                    "↑↓ · Enter use · Ctrl+S save · Esc"
                } else {
                    "↑↓ navigate · Tab complete · Enter use · Ctrl+S save as… · Esc back"
                },
            }),
            _ => None,
        }
    }

    /// Fit the whole palette above the composer. On short terminals the list
    /// takes priority over supplementary details. Selected rows stay visible.
    pub fn rows(&self, available: usize) -> Vec<(String, bool)> {
        if available == 0 {
            return Vec::new();
        }
        let show_chrome = available >= 3;
        let show_query = self.query.is_some() && available >= 4;
        let show_subtitle = self.subtitle.is_some() && available >= 7;
        let show_detail = available >= 6;
        let overhead = usize::from(show_chrome) * 2
            + usize::from(show_query)
            + usize::from(show_subtitle)
            + usize::from(show_detail);
        let count = available
            .saturating_sub(overhead)
            .min(8)
            .min(self.items.len().max(1));
        let start = self
            .selected
            .saturating_sub(count / 2)
            .min(self.items.len().saturating_sub(count));
        let mut rows = Vec::new();
        if show_chrome {
            let range = if self.items.is_empty() {
                "0 results".into()
            } else {
                format!("{}–{} / {}", start + 1, start + count, self.items.len())
            };
            rows.push((format!("{} · {range}", self.title), false));
        }
        if show_subtitle {
            rows.push((self.subtitle.clone().unwrap(), false));
        }
        if show_query {
            rows.push((self.query.clone().unwrap(), false));
        }
        if self.items.is_empty() {
            rows.push((self.empty.into(), false));
        } else {
            rows.extend(
                self.items
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(count)
                    .map(|(i, item)| {
                        (
                            format!("{} {item}", if i == self.selected { "›" } else { " " }),
                            i == self.selected,
                        )
                    }),
            );
        }
        if show_detail {
            rows.push((self.detail.clone(), false));
        }
        if show_chrome {
            rows.push((self.keys.into(), false));
        }
        rows
    }
}

/// Keep the editing caret visible even with a long pasted query on a narrow terminal.
fn query_line(label: &str, query: &str, cursor: usize, width: usize) -> String {
    let mut before: String = query.chars().take(cursor).collect();
    let after: String = query.chars().skip(cursor).collect();
    let budget = width.saturating_sub(label.len() + 6);
    let mut clipped = false;
    while crate::ui::utils::display_width(&before) > budget && !before.is_empty() {
        before.remove(0);
        clipped = true;
    }
    format!(
        "{label}: {}{before}▏{after}",
        if clipped { "…" } else { "" }
    )
}
