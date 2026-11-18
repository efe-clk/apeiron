//! The backups: the overview, backing up, the versions of a backup, what is
//! inside one, restoring and what changed.

use serde_json::{Value, json};

use super::{Status, changes, join, ref_for};
use crate::format::{self, bytes, list, num, text};
use crate::page::{Act, Item, Page, Tone, act, act_on, base_name, button, input, muted, row};
use crate::{Ctx, Nav, Screen, Step};

/// One backup of the overview.
struct Backed {
    name: String,
    source: Option<String>,
    time: String,
}

#[derive(Default)]
pub(crate) struct Overview {
    backups: Vec<Backed>,
}

impl Screen for Overview {
    fn nav(&self) -> Nav {
        Nav::Overview
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        // `folder/…` refs hold the state of synced folders (SYNC.md §3), and
        // `apeiron/…` ones are the program's own: neither is a backup.
        self.backups.clear();
        for name in super::branch_refs(cx)? {
            let page = cx.call("log", json!({"start": name, "limit": 1}))?;
            let newest = list(&page, "snapshots")
                .first()
                .cloned()
                .unwrap_or(Value::Null);
            let source = list(&newest, "meta").iter().find_map(|kv| {
                (kv.get(0)?.as_str()? == "source")
                    .then(|| kv.get(1)?.as_str().map(str::to_owned))?
            });
            self.backups.push(Backed {
                name,
                source,
                time: format::when(&newest["time"]),
            });
        }
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![Item::Heading(cx.t("overview_heading"))];
        if self.backups.is_empty() {
            items.push(Item::Text(cx.t("overview_empty"), Tone::Plain));
            items.push(Item::Buttons(vec![button(
                'b',
                cx.t("nav_backup"),
                act("new"),
            )]));
            return items;
        }
        items.push(Item::Row {
            cells: vec![cx.t("folder_of"), String::new(), cx.t("last_backup")],
            tone: Tone::Muted,
            buttons: Vec::new(),
        });
        for (n, b) in self.backups.iter().enumerate() {
            let mut buttons = vec![button('v', cx.t("history"), act_on("history", n))];
            if b.source.is_some() {
                buttons.push(button('b', cx.t("back_up_again"), act_on("again", n)));
            }
            items.push(row(
                vec![
                    b.name.trim_start_matches("backup/").to_owned(),
                    b.source.clone().unwrap_or_default(),
                    b.time.clone(),
                ],
                buttons,
            ));
        }
        items
    }

    fn act(&mut self, act: Act, _page: &mut Page, _cx: &mut Ctx) -> Step {
        match act.id {
            "new" => Step::Push(Box::new(Backup::new("", ""))),
            "history" => Step::Push(Box::new(History::new(&self.backups[act.arg].name))),
            "again" => {
                let b = &self.backups[act.arg];
                Step::Push(Box::new(Backup::new(
                    b.source.as_deref().unwrap_or_default(),
                    &b.name,
                )))
            }
            _ => Step::Stay,
        }
    }
}

pub(crate) struct Backup {
    source: String,
    name: String,
    /// The name was typed, so it no longer follows the folder.
    named: bool,
    done: Option<String>,
    status: Status,
}

impl Backup {
    pub fn new(source: &str, name: &str) -> Self {
        Backup {
            source: source.to_owned(),
            name: name.to_owned(),
            named: !name.is_empty(),
            done: None,
            status: Status::default(),
        }
    }
}

impl Screen for Backup {
    fn nav(&self) -> Nav {
        Nav::Backup
    }

    fn init(&mut self, page: &mut Page, _cx: &Ctx) {
        page.set("folder", self.source.clone());
        let name = if self.name.is_empty() && !self.source.is_empty() {
            ref_for(&self.source)
        } else {
            self.name.clone()
        };
        page.set("name", name);
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("backup_heading")),
            muted(cx.t("backup_intro")),
            Item::Input(input("folder", cx.t("path")).folder().submit(act("go"))),
            Item::Input(
                input("name", cx.t("backup_name"))
                    .hint(cx.t("backup_name_hint"))
                    .submit(act("go")),
            ),
            Item::Input(input("note", cx.t("note")).submit(act("go"))),
            Item::Buttons(vec![button('g', cx.t("backup_button"), act("go"))]),
        ];
        items.extend(self.status.items());
        if self.done.is_some() {
            items.push(Item::Buttons(vec![button(
                'v',
                cx.t("history"),
                act("history"),
            )]));
        }
        items
    }

    fn changed(&mut self, id: &'static str, page: &mut Page) {
        match id {
            "name" => self.named = true,
            "folder" if !self.named => page.set("name", ref_for(&page.value("folder"))),
            _ => {}
        }
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        match act.id {
            "go" => {
                self.status.clear();
                self.done = None;
                let path = page.value("folder");
                let note = page.value("note");
                let request = json!({
                    "source": path,
                    "ref": page.value("name"),
                    "message": if note.is_empty() { None } else { Some(note) },
                    "meta": [["source", path]],
                });
                match cx.start("backup", request) {
                    Ok(job) => page.job = Some(crate::page::Running { id: "backup", job }),
                    Err(e) => self.status.error(cx, &e),
                }
                Step::Stay
            }
            "history" => match &self.done {
                Some(name) => Step::Push(Box::new(History::new(name))),
                None => Step::Stay,
            },
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
        match result {
            Ok(r) => {
                self.status.set(
                    Tone::Good,
                    cx.tw(
                        "backup_done",
                        &[
                            ("files", num(&r, "files").to_string()),
                            ("bytes", bytes(num(&r, "bytes"), cx.w.lang)),
                            ("new_bytes", bytes(num(&r, "new_bytes"), cx.w.lang)),
                        ],
                    ),
                );
                self.status.warnings(cx, &r);
                self.done = Some(page.value("name"));
            }
            Err(e) => self.status.error(cx, &e),
        }
        Step::Stay
    }
}

/// The versions of a backup, newest first, from `from`.
pub(crate) struct History {
    name: String,
    from: String,
    snapshots: Vec<Value>,
    next: Option<String>,
}

impl History {
    pub fn new(name: &str) -> Self {
        History {
            name: name.to_owned(),
            from: name.to_owned(),
            snapshots: Vec::new(),
            next: None,
        }
    }
}

impl Screen for History {
    fn nav(&self) -> Nav {
        Nav::Overview
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        let page = cx.call("log", json!({"start": self.from, "limit": 50}))?;
        self.snapshots = list(&page, "snapshots").to_vec();
        self.next = page.get("next").and_then(Value::as_str).map(str::to_owned);
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![Item::Heading(cx.tw(
            "history_heading",
            &[("name", self.name.trim_start_matches("backup/").to_owned())],
        ))];
        for (n, s) in self.snapshots.iter().enumerate() {
            items.push(row(
                vec![format::when(&s["time"]), text(s, "message")],
                vec![
                    button('b', cx.t("browse"), act_on("browse", n)),
                    button('c', cx.t("compare"), act_on("compare", n)),
                    button('r', cx.t("restore"), act_on("restore", n)),
                ],
            ));
        }
        if self.next.is_some() {
            items.push(Item::Buttons(vec![button('n', "…", act("more"))]));
        }
        items
    }

    fn act(&mut self, act: Act, _page: &mut Page, _cx: &mut Ctx) -> Step {
        let snapshot = || self.snapshots[act.arg].clone();
        match act.id {
            "browse" => Step::Push(Box::new(Browse::new(snapshot(), String::new()))),
            "compare" => Step::Push(Box::new(Compare::new(snapshot()))),
            "restore" => Step::Push(Box::new(Restore::new(snapshot(), String::new()))),
            "more" => match &self.next {
                Some(next) => Step::Push(Box::new(History {
                    name: self.name.clone(),
                    from: next.clone(),
                    snapshots: Vec::new(),
                    next: None,
                })),
                None => Step::Stay,
            },
            _ => Step::Stay,
        }
    }
}

/// The folders and files at `path` inside a version.
pub(crate) struct Browse {
    snapshot: Value,
    path: String,
    entries: Vec<Value>,
}

impl Browse {
    fn new(snapshot: Value, path: String) -> Self {
        Browse {
            snapshot,
            path,
            entries: Vec::new(),
        }
    }

    fn full(&self, n: usize) -> String {
        let name = text(&self.entries[n], "name");
        if self.path.is_empty() {
            name
        } else {
            format!("{}/{name}", self.path)
        }
    }
}

impl Screen for Browse {
    fn nav(&self) -> Nav {
        Nav::Overview
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        let entries = cx.call(
            "list",
            json!({"snapshot": text(&self.snapshot, "id"), "path": self.path}),
        )?;
        self.entries = entries.as_array().cloned().unwrap_or_default();
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let time = format::when(&self.snapshot["time"]);
        let mut items = vec![
            Item::Heading(cx.tw("browse_heading", &[("time", time)])),
            muted(format!("/ {}", self.path.replace('/', " / "))),
        ];
        if self.entries.is_empty() {
            items.push(muted(cx.t("empty_folder")));
        }
        for (n, e) in self.entries.iter().enumerate() {
            let kind = text(e, "kind");
            let icon = match kind.as_str() {
                "dir" => "📁",
                "symlink" => "🔗",
                _ => "📄",
            };
            let size = e
                .get("size")
                .and_then(Value::as_u64)
                .map(|s| bytes(s, cx.w.lang))
                .unwrap_or_default();
            let mut buttons = Vec::new();
            if kind == "dir" {
                buttons.push(button('o', cx.t("browse"), act_on("open", n)));
            }
            buttons.push(button('r', cx.t("restore_this"), act_on("restore", n)));
            items.push(row(
                vec![
                    format!("{icon} {}", text(e, "name")),
                    size,
                    format::when(&e["mtime"]),
                ],
                buttons,
            ));
        }
        items
    }

    fn act(&mut self, act: Act, _page: &mut Page, _cx: &mut Ctx) -> Step {
        match act.id {
            "open" => Step::Push(Box::new(Browse::new(
                self.snapshot.clone(),
                self.full(act.arg),
            ))),
            "restore" => Step::Push(Box::new(Restore::new(
                self.snapshot.clone(),
                self.full(act.arg),
            ))),
            _ => Step::Stay,
        }
    }
}

/// Restores a version, or `path` in it, into a new folder.
pub(crate) struct Restore {
    snapshot: Value,
    path: String,
    status: Status,
}

impl Restore {
    fn new(snapshot: Value, path: String) -> Self {
        Restore {
            snapshot,
            path,
            status: Status::default(),
        }
    }

    fn target(&self, page: &Page) -> String {
        let name = page.value("new_name");
        let name = name.trim();
        let target = page.value("target");
        if name.is_empty() {
            target
        } else {
            join(&target, name)
        }
    }
}

impl Screen for Restore {
    fn nav(&self) -> Nav {
        Nav::Overview
    }

    fn init(&mut self, page: &mut Page, _cx: &Ctx) {
        let base = base_name(&self.path);
        let base = if base.is_empty() {
            "apeiron".to_owned()
        } else {
            base
        };
        let date = format::date(format::secs(&self.snapshot["time"]));
        page.set("new_name", format!("{base} {date}"));
        if let Some(home) = dirs::home_dir() {
            page.set("target", home.display().to_string());
        }
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let time = format::when(&self.snapshot["time"]);
        let what = if self.path.is_empty() {
            cx.tw("restore_what_all", &[("time", time)])
        } else {
            cx.tw(
                "restore_what_path",
                &[("path", self.path.clone()), ("time", time)],
            )
        };
        let mut items = vec![
            Item::Heading(cx.t("restore_heading")),
            Item::Text(what, Tone::Plain),
            muted(cx.t("restore_intro")),
            Item::Input(
                input("target", cx.t("restore_target"))
                    .folder()
                    .submit(act("go")),
            ),
            Item::Input(input("new_name", cx.t("new_folder_name")).submit(act("go"))),
            Item::Buttons(vec![button('g', cx.t("restore_button"), act("go"))]),
        ];
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        if act.id == "go" {
            self.status.clear();
            let request = json!({
                "snapshot": text(&self.snapshot, "id"),
                "target": self.target(page),
                "path": self.path,
            });
            match cx.start("restore", request) {
                Ok(job) => page.job = Some(crate::page::Running { id: "restore", job }),
                Err(e) => self.status.error(cx, &e),
            }
        }
        Step::Stay
    }

    fn done(
        &mut self,
        _id: &'static str,
        result: apeiron::Result<Value>,
        page: &mut Page,
        cx: &mut Ctx,
    ) -> Step {
        match result {
            Ok(r) => {
                self.status.set(
                    Tone::Good,
                    cx.tw(
                        "restore_done",
                        &[
                            ("files", num(&r, "files").to_string()),
                            ("bytes", bytes(num(&r, "bytes"), cx.w.lang)),
                        ],
                    ),
                );
                self.status.push(Tone::Muted, self.target(page));
                self.status.warnings(cx, &r);
            }
            Err(e) => self.status.error(cx, &e),
        }
        Step::Stay
    }
}

/// What changed in a version against the one before it.
pub(crate) struct Compare {
    snapshot: Value,
    changes: Option<Vec<Value>>,
}

impl Compare {
    fn new(snapshot: Value) -> Self {
        Compare {
            snapshot,
            changes: None,
        }
    }
}

impl Screen for Compare {
    fn nav(&self) -> Nav {
        Nav::Overview
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        if let Some(parent) = list(&self.snapshot, "parents").first() {
            let found = cx.call(
                "diff_text",
                json!({"from": parent, "to": text(&self.snapshot, "id")}),
            )?;
            self.changes = Some(found.as_array().cloned().unwrap_or_default());
        }
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let time = format::when(&self.snapshot["time"]);
        let mut items = vec![Item::Heading(cx.tw("compare_heading", &[("time", time)]))];
        match &self.changes {
            None => items.push(Item::Text(cx.t("compare_first"), Tone::Plain)),
            Some(c) if c.is_empty() => items.push(Item::Text(cx.t("compare_none"), Tone::Plain)),
            Some(c) => items.extend(changes(cx, c)),
        }
        items
    }

    fn act(&mut self, _act: Act, _page: &mut Page, _cx: &mut Ctx) -> Step {
        Step::Stay
    }
}
