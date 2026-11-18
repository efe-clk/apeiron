//! Remotes, a sync round of a folder and its conflict copies (SYNC.md).

use serde_json::{Value, json};

use super::Status;
use crate::format::{list, text};
use crate::page::{Act, Item, Page, Running, Tone, act, act_on, button, input, muted, row};
use crate::{Ctx, Nav, Screen, Step};

#[derive(Default)]
pub(crate) struct Sync {
    remotes: Vec<Value>,
    status: Status,
    /// The conflict copies found in the folder, when looked for.
    conflicts: Option<(String, Vec<Value>)>,
    remote_status: Status,
}

impl Sync {
    fn find_conflicts(&mut self, cx: &Ctx, folder: String) {
        match cx.call("conflicts", json!({"folder": folder})) {
            Ok(found) => {
                self.conflicts = Some((folder, found.as_array().cloned().unwrap_or_default()));
            }
            Err(e) => self.status.push(Tone::Bad, cx.error(&e)),
        }
    }
}

impl Screen for Sync {
    fn nav(&self) -> Nav {
        Nav::Sync
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        self.remotes = cx
            .call("remotes", Value::Null)?
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("sync_heading")),
            muted(cx.t("sync_intro")),
        ];
        if self.remotes.is_empty() {
            items.push(Item::Text(cx.t("sync_no_remote"), Tone::Plain));
        } else {
            items.extend([
                Item::Input(
                    input("folder", cx.t("sync_folder"))
                        .folder()
                        .submit(act("sync")),
                ),
                Item::Select {
                    id: "remote",
                    label: cx.t("sync_remote"),
                    options: self
                        .remotes
                        .iter()
                        .map(|r| (text(r, "name"), text(r, "name")))
                        .collect(),
                },
                Item::Select {
                    id: "mode",
                    label: cx.t("sync_mode"),
                    options: ["two-way", "send", "receive"]
                        .iter()
                        .map(|m| (cx.t(&format!("sync_mode_{m}")), (*m).to_owned()))
                        .collect(),
                },
                Item::Check {
                    id: "dry_run",
                    label: cx.t("sync_dry_run"),
                },
                Item::Buttons(vec![
                    button('s', cx.t("sync_button"), act("sync")),
                    button('f', cx.t("conflicts_find"), act("find")),
                ]),
            ]);
        }
        items.extend(self.status.items());
        if let Some((_, found)) = &self.conflicts {
            items.push(Item::Heading(cx.t("conflicts_heading")));
            if found.is_empty() {
                items.push(Item::Text(cx.t("conflicts_none"), Tone::Good));
            } else {
                items.push(muted(cx.t("conflicts_intro")));
                for (n, c) in found.iter().enumerate() {
                    let exists = c["exists"].as_bool().unwrap_or(false);
                    let mut file = text(c, "path");
                    if !exists {
                        file = format!("{file} ({})", cx.t("conflicts_missing"));
                    }
                    let keep_this = if exists {
                        "conflicts_keep_this"
                    } else {
                        "conflicts_keep_none"
                    };
                    items.push(row(
                        vec![file, text(c, "copy")],
                        vec![
                            button('t', cx.t(keep_this), act_on("keep_this", n)),
                            button('c', cx.t("conflicts_keep_copy"), act_on("keep_copy", n)),
                        ],
                    ));
                }
            }
        }
        items.push(Item::Heading(cx.t("remotes_heading")));
        for (n, r) in self.remotes.iter().enumerate() {
            items.push(row(
                vec![text(r, "name"), text(r, "url")],
                vec![button('x', cx.t("remove"), act_on("remove", n))],
            ));
        }
        items.extend([
            Item::Input(
                input("name", cx.t("remote_name"))
                    .hint(cx.t("remote_name_hint"))
                    .submit(act("add")),
            ),
            Item::Input(
                input("url", cx.t("remote_url"))
                    .hint(cx.t("remote_url_hint"))
                    .submit(act("add")),
            ),
            Item::Buttons(vec![button('a', cx.t("remote_add"), act("add"))]),
        ]);
        items.extend(self.remote_status.items());
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        match act.id {
            "sync" => {
                self.status.clear();
                self.conflicts = None;
                let request = json!({
                    "folder": page.value("folder"),
                    "remote": page.choice("remote"),
                    "mode": page.choice("mode"),
                    "dry_run": page.checked("dry_run"),
                });
                match cx.start("sync", request) {
                    Ok(job) => page.job = Some(Running { id: "sync", job }),
                    Err(e) => self.status.error(cx, &e),
                }
                Step::Stay
            }
            "find" => {
                self.status.clear();
                self.find_conflicts(cx, page.value("folder"));
                Step::Stay
            }
            "keep_this" | "keep_copy" => {
                let Some((folder, found)) = &self.conflicts else {
                    return Step::Stay;
                };
                let folder = folder.clone();
                let keep = if act.id == "keep_this" {
                    "this"
                } else {
                    "copy"
                };
                let request =
                    json!({"folder": folder, "copy": text(&found[act.arg], "copy"), "keep": keep});
                match cx.call("resolve_conflict", request) {
                    Ok(_) => self.find_conflicts(cx, folder),
                    Err(e) => self.status.error(cx, &e),
                }
                Step::Stay
            }
            "remove" => {
                let name = text(&self.remotes[act.arg], "name");
                page.ask(
                    cx.tw("remote_remove_ask", &[("name", name)]),
                    act_on("remove_sure", act.arg),
                );
                Step::Stay
            }
            "remove_sure" => {
                self.remote_status.clear();
                let name = text(&self.remotes[act.arg], "name");
                match cx.call("remove_remote", json!({"name": name})) {
                    Ok(_) => Step::Reload,
                    Err(e) => {
                        self.remote_status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            "add" => {
                self.remote_status.clear();
                let request = json!({
                    "name": page.value("name").trim(),
                    "url": page.value("url").trim(),
                });
                match cx.call("add_remote", request) {
                    Ok(_) => {
                        page.set("name", "");
                        page.set("url", "");
                        Step::Reload
                    }
                    Err(e) => {
                        self.remote_status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            _ => Step::Stay,
        }
    }

    fn done(
        &mut self,
        _id: &'static str,
        result: apeiron::Result<Value>,
        page: &mut Page,
        cx: &mut Ctx,
    ) -> Step {
        let r = match result {
            Ok(r) => r,
            Err(e) => {
                self.status.error(cx, &e);
                return Step::Stay;
            }
        };
        let changes = list(&r, "changes");
        let count = [("changes", changes.len().to_string())];
        let summary = if page.checked("dry_run") {
            cx.tw("sync_would", &count)
        } else {
            cx.tw(&format!("sync_{}", text(&r, "action")), &count)
        };
        self.status.set(Tone::Good, summary);
        for c in changes {
            self.status.push(
                Tone::Plain,
                format!("  {}  {}", cx.t(&text(c, "kind")), text(c, "path")),
            );
        }
        let conflicts = list(&r, "conflicts");
        if !conflicts.is_empty() {
            self.status.push(Tone::Bad, cx.t("sync_conflicts"));
            for c in conflicts {
                self.status.push(
                    Tone::Bad,
                    cx.tw(
                        &format!("conflict_{}", text(c, "kind")),
                        &[("path", text(c, "path")), ("copy", text(c, "copy"))],
                    ),
                );
            }
        }
        let skipped = list(&r, "skipped");
        if !skipped.is_empty() {
            self.status.push(Tone::Plain, cx.t("sync_skipped"));
            for p in skipped {
                let p = match p {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                self.status.push(Tone::Plain, format!("  {p}"));
            }
        }
        self.status.warnings(cx, &r);
        let folder = page.value("folder");
        self.find_conflicts(cx, folder);
        Step::Stay
    }
}
