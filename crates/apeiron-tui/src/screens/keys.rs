//! Passwords and recovery keys (FORMAT.md §14), and plugins (PLUGIN.md §1).

use serde_json::{Value, json};

use super::Status;
use crate::format::text;
use crate::page::{Act, Item, Page, Tone, act, act_on, button, input, muted, row};
use crate::{Ctx, Nav, Screen, Step};

#[derive(Default)]
pub(crate) struct Keys {
    encrypted: bool,
    keys: Vec<Value>,
    status: Status,
}

impl Screen for Keys {
    fn nav(&self) -> Nav {
        Nav::Keys
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        let info = cx.call("info", Value::Null)?;
        self.encrypted = info["encrypted"].as_bool().unwrap_or(false);
        self.keys = if self.encrypted {
            cx.call("keys", Value::Null)?
                .as_array()
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![Item::Heading(cx.t("keys_heading"))];
        if !self.encrypted {
            items.push(Item::Text(cx.t("keys_not_encrypted"), Tone::Plain));
            return items;
        }
        items.push(muted(cx.t("keys_intro")));
        items.push(Item::Row {
            cells: vec![cx.t("keys_kind"), cx.t("keys_created"), cx.t("keys_id")],
            tone: Tone::Muted,
            buttons: Vec::new(),
        });
        for (n, k) in self.keys.iter().enumerate() {
            let mut kind = cx.t(&format!("keys_kind_{}", text(k, "kind")));
            if k["current"].as_bool().unwrap_or(false) {
                kind = format!("{kind} ({})", cx.t("keys_current"));
            }
            let buttons = if self.keys.len() > 1 {
                vec![button('x', cx.t("remove"), act_on("remove", n))]
            } else {
                Vec::new()
            };
            items.push(row(vec![kind, text(k, "created"), text(k, "id")], buttons));
        }
        items.extend([
            Item::Heading(cx.t("keys_add")),
            Item::Input(
                input("password", cx.t("password"))
                    .hint(cx.t("keys_password_hint"))
                    .secret()
                    .submit(act("password")),
            ),
            Item::Input(
                input("again", cx.t("password_again"))
                    .secret()
                    .submit(act("password")),
            ),
            Item::Buttons(vec![
                button('p', cx.t("keys_add_password"), act("password")),
                button('k', cx.t("keys_add_recovery"), act("recovery")),
            ]),
        ]);
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        match act.id {
            "remove" => {
                page.ask(cx.t("keys_remove_ask"), act_on("remove_sure", act.arg));
                Step::Stay
            }
            "remove_sure" => {
                self.status.clear();
                let id = text(&self.keys[act.arg], "id");
                match cx.call("remove_key", json!({"id": id})) {
                    Ok(_) => Step::Reload,
                    Err(e) => {
                        self.status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            "password" => {
                self.status.clear();
                let password = page.value("password");
                if password != page.value("again") {
                    self.status.set(Tone::Bad, cx.t("passwords_differ"));
                    return Step::Stay;
                }
                match cx.call("add_password", json!({"password": password})) {
                    Ok(_) => {
                        page.set("password", "");
                        page.set("again", "");
                        self.status.set(Tone::Good, cx.t("keys_password_added"));
                        Step::Reload
                    }
                    Err(e) => {
                        self.status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            "recovery" => {
                self.status.clear();
                match cx.call("add_recovery_key", Value::Null) {
                    Ok(added) => {
                        self.status.push(Tone::Plain, cx.t("recovery_heading"));
                        self.status.push(Tone::Plain, cx.t("recovery_text"));
                        self.status.push(Tone::Key, text(&added, "recovery_key"));
                        Step::Reload
                    }
                    Err(e) => {
                        self.status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            _ => Step::Stay,
        }
    }
}

#[derive(Default)]
pub(crate) struct Plugins {
    plugins: Vec<Value>,
    status: Status,
}

/// A request to a plugin operation, in the plugin folder of the options.
fn with_dir(cx: &Ctx, mut request: Value) -> Value {
    if let (Some(dir), Some(o)) = (&cx.plugins, request.as_object_mut()) {
        o.insert("dir".to_owned(), json!(dir));
    }
    request
}

impl Screen for Plugins {
    fn nav(&self) -> Nav {
        Nav::Plugins
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        self.plugins = cx
            .call("plugins", with_dir(cx, json!({})))?
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("plugins_heading")),
            muted(cx.t("plugins_intro")),
        ];
        if self.plugins.is_empty() {
            items.push(Item::Text(cx.t("plugins_none"), Tone::Plain));
        } else {
            items.push(Item::Row {
                cells: vec![
                    cx.t("plugins_name"),
                    cx.t("plugins_description"),
                    cx.t("plugins_patterns"),
                ],
                tone: Tone::Muted,
                buttons: Vec::new(),
            });
        }
        for (n, p) in self.plugins.iter().enumerate() {
            let patterns: Vec<String> = crate::format::list(p, "patterns")
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            items.push(row(
                vec![
                    format!("{} {}", text(p, "name"), text(p, "version")),
                    text(p, "description"),
                    patterns.join(", "),
                ],
                vec![button('x', cx.t("remove"), act_on("remove", n))],
            ));
        }
        items.extend([
            Item::Heading(cx.t("plugins_add")),
            Item::Input(
                input("file", cx.t("plugins_file"))
                    .hint(cx.t("plugins_file_hint"))
                    .submit(act("add")),
            ),
            Item::Buttons(vec![button('a', cx.t("plugins_add"), act("add"))]),
        ]);
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        match act.id {
            "remove" => {
                let name = text(&self.plugins[act.arg], "name");
                page.ask(
                    cx.tw("plugins_remove_ask", &[("name", name)]),
                    act_on("remove_sure", act.arg),
                );
                Step::Stay
            }
            "remove_sure" => {
                self.status.clear();
                let name = text(&self.plugins[act.arg], "name");
                match cx.call("plugin_remove", with_dir(cx, json!({"name": name}))) {
                    Ok(_) => Step::Reload,
                    Err(e) => {
                        self.status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            "add" => {
                self.status.clear();
                let file = page.value("file").trim().to_owned();
                match cx.call("plugin_add", with_dir(cx, json!({"file": file}))) {
                    Ok(added) => {
                        let key = if added["replaced"].as_bool().unwrap_or(false) {
                            "plugins_replaced"
                        } else {
                            "plugins_added"
                        };
                        let name = text(&added["info"], "name");
                        self.status.set(Tone::Good, cx.tw(key, &[("name", name)]));
                        page.set("file", "");
                        Step::Reload
                    }
                    Err(e) => {
                        self.status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            _ => Step::Stay,
        }
    }
}
