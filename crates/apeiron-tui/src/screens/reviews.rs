//! Reviews (REVIEW.md): the list, asking for one, and one with its steps.

use serde_json::{Value, json};

use super::{Status, branch_refs, changes};
use crate::format::{self, list, text};
use crate::page::{Act, Item, Page, Tone, act, act_on, button, input, muted, row};
use crate::{Ctx, Nav, Screen, Step};

fn state(cx: &Ctx, r: &Value) -> String {
    let s = text(r, "state");
    if s == "open" && r["ready"].as_bool().unwrap_or(false) {
        cx.t("review_ready")
    } else {
        cx.t(&format!("review_{s}"))
    }
}

#[derive(Default)]
pub(crate) struct Reviews {
    all: bool,
    reviews: Vec<Value>,
    names: Vec<String>,
    status: Status,
}

impl Screen for Reviews {
    fn nav(&self) -> Nav {
        Nav::Reviews
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        self.reviews = cx
            .call("reviews", json!({"all": self.all}))?
            .as_array()
            .cloned()
            .unwrap_or_default();
        self.names = branch_refs(cx)?;
        Ok(())
    }

    fn init(&mut self, page: &mut Page, cx: &Ctx) {
        let items = self.build(cx, page);
        page.show(items);
        if let Some(from) = self.names.iter().find(|n| *n != "main") {
            page.choose("from", from);
        }
        if self.names.iter().any(|n| n == "main") {
            page.choose("into", "main");
        }
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("reviews_heading")),
            muted(cx.t("reviews_intro")),
        ];
        if self.reviews.is_empty() {
            let key = if self.all {
                "reviews_none_all"
            } else {
                "reviews_none"
            };
            items.push(Item::Text(cx.t(key), Tone::Plain));
        }
        for (n, r) in self.reviews.iter().enumerate() {
            items.push(row(
                vec![
                    text(r, "title"),
                    format!(
                        "{} → {} · {}",
                        text(r, "from"),
                        text(r, "into"),
                        text(r, "author")
                    ),
                    state(cx, r),
                    format::seconds(r["opened"].as_i64().unwrap_or(0)),
                ],
                vec![button('o', cx.t("review_open_button"), act_on("open", n))],
            ));
        }
        let toggle = if self.all {
            "reviews_open_only"
        } else {
            "reviews_all"
        };
        items.push(Item::Buttons(vec![button(
            'l',
            cx.t(toggle),
            act("toggle"),
        )]));
        if self.names.len() > 1 {
            let options: Vec<(String, String)> =
                self.names.iter().map(|n| (n.clone(), n.clone())).collect();
            items.extend([
                Item::Heading(cx.t("review_ask")),
                Item::Select {
                    id: "from",
                    label: format!("{} ({})", cx.t("review_from"), cx.t("review_from_hint")),
                    options: options.clone(),
                },
                Item::Select {
                    id: "into",
                    label: cx.t("review_into"),
                    options,
                },
                Item::Input(input("title", cx.t("review_title")).submit(act("ask"))),
                Item::Input(input("text", cx.t("review_text")).submit(act("ask"))),
                Item::Buttons(vec![button('s', cx.t("review_ask"), act("ask"))]),
            ]);
        }
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        match act.id {
            "open" => Step::Push(Box::new(ReviewView::new(text(
                &self.reviews[act.arg],
                "id",
            )))),
            "toggle" => {
                self.all = !self.all;
                Step::Reload
            }
            "ask" => {
                self.status.clear();
                let body = page.value("text");
                let request = json!({
                    "from": page.choice("from"),
                    "into": page.choice("into"),
                    "title": page.value("title"),
                    "text": if body.is_empty() { None } else { Some(body) },
                });
                match cx.call("open_review", request) {
                    Ok(made) => {
                        page.set("title", "");
                        page.set("text", "");
                        Step::Push(Box::new(ReviewView::new(text(&made, "id"))))
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

/// One review with its steps and what its branch changes.
pub(crate) struct ReviewView {
    id: String,
    review: Value,
    changes: Vec<Value>,
    status: Status,
}

impl ReviewView {
    fn new(id: String) -> Self {
        ReviewView {
            id,
            review: Value::Null,
            changes: Vec::new(),
            status: Status::default(),
        }
    }
}

impl Screen for ReviewView {
    fn nav(&self) -> Nav {
        Nav::Reviews
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        self.review = cx.call("review", json!({"id": self.id}))?;
        let snapshot = text(&self.review, "snapshot");
        self.changes = if snapshot.is_empty() {
            Vec::new()
        } else {
            cx.call(
                "diff_text",
                json!({"from": text(&self.review, "into"), "to": snapshot}),
            )?
            .as_array()
            .cloned()
            .unwrap_or_default()
        };
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let r = &self.review;
        let who = |key: &str| {
            let names: Vec<String> = list(r, key)
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if names.is_empty() {
                cx.t("review_no_one")
            } else {
                names.join(", ")
            }
        };
        let mut items = vec![
            Item::Heading(text(r, "title")),
            muted(format!(
                "{} → {} · {} · {}",
                text(r, "from"),
                text(r, "into"),
                text(r, "author"),
                state(cx, r)
            )),
            Item::Text(
                format!("{}: {}", cx.t("review_approved_by"), who("approvals")),
                Tone::Plain,
            ),
        ];
        if !list(r, "changes_requested").is_empty() {
            items.push(Item::Text(
                format!(
                    "{}: {}",
                    cx.t("review_changes_by"),
                    who("changes_requested")
                ),
                Tone::Plain,
            ));
        }
        items.push(Item::Gap);
        for e in list(r, "events") {
            let mut head = format!(
                "{} {}",
                text(e, "author"),
                cx.t(&format!("review_did_{}", text(e, "action")))
            );
            let path = text(e, "path");
            if !path.is_empty() {
                head = format!("{head} · {path}");
            }
            head = format!(
                "{head} · {}",
                format::seconds(e["time"].as_i64().unwrap_or(0))
            );
            items.push(Item::Text(head, Tone::Muted));
            let body = text(e, "text");
            if !body.is_empty() {
                items.push(Item::Text(body, Tone::Plain));
            }
        }
        let open = text(r, "state") == "open";
        items.push(Item::Gap);
        items.push(Item::Input(input("text", cx.t("review_text"))));
        if !self.changes.is_empty() {
            let mut options = vec![(cx.t("review_whole"), String::new())];
            options.extend(self.changes.iter().map(|c| {
                let p = text(c, "path");
                (p.clone(), p)
            }));
            items.push(Item::Select {
                id: "about",
                label: cx.t("review_about"),
                options,
            });
        }
        let mut buttons = vec![button('c', cx.t("review_comment"), act("comment"))];
        if open {
            buttons.extend([
                button('a', cx.t("review_approve"), act("approve")),
                button('r', cx.t("review_request_changes"), act("request_changes")),
                button('x', cx.t("review_close"), act("close")),
            ]);
            if r["ready"].as_bool().unwrap_or(false) {
                buttons.push(button('m', cx.t("review_merge"), act("merge")));
            }
        }
        items.push(Item::Buttons(buttons));
        items.extend(self.status.items());
        if open {
            items.push(Item::Heading(
                cx.tw("review_changes_heading", &[("into", text(r, "into"))]),
            ));
            if self.changes.is_empty() {
                items.push(Item::Text(cx.t("compare_none"), Tone::Plain));
            } else {
                items.extend(changes(cx, &self.changes));
            }
        }
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        self.status.clear();
        let id = text(&self.review, "id");
        let result = match act.id {
            "merge" => {
                let question = cx.tw(
                    "review_merge_ask",
                    &[
                        ("from", text(&self.review, "from")),
                        ("into", text(&self.review, "into")),
                    ],
                );
                page.ask(question, crate::page::act("merge_sure"));
                return Step::Stay;
            }
            "merge_sure" => cx.call("merge_review", json!({"id": id})),
            action => {
                let body = page.value("text");
                let about = page.choice("about");
                cx.call(
                    "note_review",
                    json!({
                        "id": id,
                        "action": action,
                        "text": if body.is_empty() { None } else { Some(body) },
                        "path": if action == "comment" && !about.is_empty() { Some(about) } else { None },
                    }),
                )
            }
        };
        match result {
            Ok(_) => {
                page.set("text", "");
                Step::Reload
            }
            Err(e) => {
                self.status.error(cx, &e);
                Step::Stay
            }
        }
    }
}
