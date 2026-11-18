//! The screens, one for each of the desktop interface's (DESKTOP.md §2).

mod backups;
mod check;
mod keys;
mod reviews;
mod schedule;
mod start;
mod sync;

pub(crate) use backups::{Backup, Overview};
pub(crate) use check::{Report, Retention, Verify};
pub(crate) use keys::{Keys, Plugins};
pub(crate) use reviews::Reviews;
pub(crate) use schedule::Schedule;
pub(crate) use start::Start;
pub(crate) use sync::Sync;

use ratatui::style::{Color, Style};
use ratatui::text::Line;
use serde_json::Value;

use crate::Ctx;
use crate::format;
use crate::page::{Item, Tone};

/// Messages a screen shows under its form after an action.
#[derive(Default)]
pub(crate) struct Status(Vec<(Tone, String)>);

impl Status {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn push(&mut self, tone: Tone, text: impl Into<String>) {
        self.0.push((tone, text.into()));
    }

    pub fn set(&mut self, tone: Tone, text: impl Into<String>) {
        self.clear();
        self.push(tone, text);
    }

    pub fn error(&mut self, cx: &Ctx, e: &apeiron::Error) {
        self.set(Tone::Bad, cx.error(e));
    }

    pub fn items(&self) -> impl Iterator<Item = Item> + '_ {
        self.0.iter().map(|(t, s)| Item::Text(s.clone(), *t))
    }

    /// The skipped files of a result, as the pages list them.
    pub fn warnings(&mut self, cx: &Ctx, result: &Value) {
        let warnings = format::list(result, "warnings");
        if !warnings.is_empty() {
            self.push(Tone::Bad, cx.t("warnings"));
            for w in warnings {
                self.push(
                    Tone::Bad,
                    format!(
                        "{}: {}",
                        format::text(w, "path"),
                        format::text(w, "message")
                    ),
                );
            }
        }
    }
}

/// A ref name from a folder name: lowercase letters, digits, `-` and `_`
/// (FORMAT.md §9.3), as the pages make it.
pub(crate) fn ref_for(folder: &str) -> String {
    let name: String = crate::page::base_name(folder)
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '-' => c,
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ğ' => 'g',
            'ı' | 'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ş' => 's',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            _ => '-',
        })
        .collect();
    let mut squeezed = String::new();
    for c in name.chars() {
        if !(c == '-' && squeezed.ends_with('-')) {
            squeezed.push(c);
        }
    }
    let name = squeezed.trim_matches('-');
    format!("backup/{}", if name.is_empty() { "folder" } else { name })
}

/// `name` inside the folder `parent`.
pub(crate) fn join(parent: &str, name: &str) -> String {
    let sep = if parent.contains('\\') && !parent.contains('/') {
        '\\'
    } else {
        '/'
    };
    if parent.ends_with(sep) {
        format!("{parent}{name}")
    } else {
        format!("{parent}{sep}{name}")
    }
}

/// Refs that hold backups or branches, not the program's own or synced
/// folders.
pub(crate) fn branch_refs(cx: &Ctx) -> apeiron::Result<Vec<String>> {
    let refs = cx.call("refs", Value::Null)?;
    Ok(refs
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|r| format::text(r, "name"))
        .filter(|n| !n.starts_with("folder/") && !n.starts_with("apeiron/"))
        .collect())
}

/// A unified diff, with added, removed and hunk lines in their colors.
pub(crate) fn diff_lines(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| {
            let style = if l.starts_with("@@") {
                Style::default().fg(Color::Cyan)
            } else if l.starts_with('+') {
                Style::default().fg(Color::Green)
            } else if l.starts_with('-') {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            };
            Line::styled(l.replace('\t', "    "), style)
        })
        .collect()
}

/// The changes of `diff_text`, each with its kind and changed lines.
pub(crate) fn changes(cx: &Ctx, changes: &[Value]) -> Vec<Item> {
    let mut items = Vec::new();
    for c in changes {
        items.push(Item::Gap);
        items.push(Item::Text(
            format!(
                "[{}] {}",
                cx.t(&format::text(c, "kind")),
                format::text(c, "path")
            ),
            Tone::Plain,
        ));
        if let Some(text) = c.get("text").and_then(Value::as_str) {
            items.push(Item::Pre(diff_lines(text)));
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_made_from_folder_names() {
        assert_eq!(ref_for("/home/a/Belgeler"), "backup/belgeler");
        assert_eq!(
            ref_for("/home/a/Müzik Çalışmaları/"),
            "backup/muzik-calismalari"
        );
        assert_eq!(ref_for("C:\\Users\\a\\My Photos"), "backup/my-photos");
        assert_eq!(ref_for(""), "backup/folder");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("C:\\x\\", "b"), "C:\\x\\b");
    }
}
