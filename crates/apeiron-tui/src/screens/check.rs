//! Checking the repository, the compliance report and the retention rules.

use serde_json::{Value, json};

use super::Status;
use crate::format::{self, list, num, text};
use crate::page::{Act, Item, Page, Running, Tone, act, button, input, muted, row};
use crate::{Ctx, Nav, Screen, Step};

#[derive(Default)]
pub(crate) struct Verify {
    status: Status,
}

impl Screen for Verify {
    fn nav(&self) -> Nav {
        Nav::Verify
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("verify_heading")),
            muted(cx.t("verify_intro")),
            Item::Buttons(vec![button('g', cx.t("verify_button"), act("go"))]),
        ];
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, _act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        self.status.clear();
        match cx.start("verify", json!({})) {
            Ok(job) => page.job = Some(Running { id: "verify", job }),
            Err(e) => self.status.error(cx, &e),
        }
        Step::Stay
    }

    fn done(
        &mut self,
        _id: &'static str,
        result: apeiron::Result<Value>,
        _page: &mut Page,
        cx: &mut Ctx,
    ) -> Step {
        match result {
            Ok(r) => {
                let damaged = list(&r, "damaged_files").len() + list(&r, "damaged_objects").len();
                if damaged > 0 {
                    self.status.set(
                        Tone::Bad,
                        cx.tw("verify_bad", &[("count", damaged.to_string())]),
                    );
                } else {
                    self.status.set(
                        Tone::Good,
                        cx.tw(
                            "verify_ok",
                            &[("objects", num(&r, "objects_checked").to_string())],
                        ),
                    );
                }
            }
            Err(e) => self.status.error(cx, &e),
        }
        Step::Stay
    }
}

#[derive(Default)]
pub(crate) struct Report {
    report: Option<Value>,
    status: Status,
}

/// A finding in the user's language; `text` is English (REPORTS.md §4).
fn finding(cx: &Ctx, f: &Value) -> String {
    let code = text(f, "code");
    let key = if code == "copy" {
        format!("finding_copy_{}", text(f, "state"))
    } else {
        format!("finding_{code}")
    };
    if !cx.w.has(&key) {
        return text(f, "text");
    }
    let values: Vec<(&str, String)> = f
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.as_str(), v)
                })
                .collect()
        })
        .unwrap_or_default();
    cx.tw(&key, &values)
}

impl Report {
    fn view(&self, cx: &Ctx, r: &Value) -> Vec<Item> {
        let mut items = vec![Item::Heading(cx.t("report_findings"))];
        let findings = list(r, "findings");
        if findings.is_empty() {
            items.push(Item::Text(cx.t("report_no_findings"), Tone::Good));
        }
        for f in findings {
            let tone = if text(f, "level") == "problem" {
                Tone::Bad
            } else {
                Tone::Plain
            };
            items.push(Item::Text(format!("• {}", finding(cx, f)), tone));
        }
        let repo = &r["repository"];
        let encrypted = repo["encrypted"].as_bool().unwrap_or(false);
        let yes_no = |b: bool| cx.t(if b { "yes" } else { "no" });
        let mut facts = vec![(cx.t("report_encrypted"), yes_no(encrypted))];
        if encrypted {
            facts.push((cx.t("report_passwords"), num(repo, "passwords").to_string()));
            facts.push((
                cx.t("report_recovery_keys"),
                num(repo, "recovery_keys").to_string(),
            ));
        }
        let verified = match r.get("verify").filter(|v| !v.is_null()) {
            Some(v) => cx.tw(
                "report_verified_result",
                &[
                    ("objects", num(v, "objects_checked").to_string()),
                    (
                        "damaged",
                        (list(v, "damaged_files").len() + list(v, "damaged_objects").len())
                            .to_string(),
                    ),
                ],
            ),
            None => cx.t("report_not_verified"),
        };
        facts.push((cx.t("report_verified"), verified));
        items.push(Item::Heading(cx.t("report_repository")));
        for (k, v) in facts {
            items.push(row(vec![k, v], Vec::new()));
        }
        items.push(Item::Heading(cx.t("report_backups")));
        items.push(Item::Row {
            cells: [
                "report_ref",
                "report_snapshot",
                "report_newest",
                "report_snapshots",
                "report_kept",
                "report_copies",
            ]
            .iter()
            .map(|k| cx.t(k))
            .collect(),
            tone: Tone::Muted,
            buttons: Vec::new(),
        });
        for f in list(r, "refs") {
            let mut name = text(f, "name");
            if f["stale"].as_bool().unwrap_or(false) {
                name = format!("{name} ({})", cx.t("report_stale"));
            }
            let kept = f
                .get("retention_days")
                .and_then(Value::as_u64)
                .map(|d| cx.tw("report_days", &[("days", d.to_string())]))
                .unwrap_or_else(|| "—".to_owned());
            let copies = list(f, "copies")
                .iter()
                .map(|c| {
                    format!(
                        "{}: {}",
                        text(c, "remote"),
                        cx.t(&format!("copy_{}", text(c, "state")))
                    )
                })
                .collect::<Vec<_>>();
            let copies = if copies.is_empty() {
                "—".to_owned()
            } else {
                copies.join(", ")
            };
            items.push(Item::Row {
                cells: vec![
                    name,
                    text(f, "snapshot").chars().take(12).collect(),
                    format::when(&f["time"]),
                    num(f, "snapshots").to_string(),
                    kept,
                    copies,
                ],
                tone: if f["stale"].as_bool().unwrap_or(false) {
                    Tone::Bad
                } else {
                    Tone::Plain
                },
                buttons: Vec::new(),
            });
        }
        items.push(Item::Gap);
        items.push(muted(cx.t("report_proof")));
        items
    }
}

impl Screen for Report {
    fn nav(&self) -> Nav {
        Nav::Report
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("report_heading")),
            muted(cx.t("report_intro")),
            Item::Check {
                id: "verify",
                label: cx.t("report_verify"),
            },
            Item::Check {
                id: "remotes",
                label: cx.t("report_remotes"),
            },
            Item::Buttons(vec![button('g', cx.t("report_button"), act("go"))]),
        ];
        items.extend(self.status.items());
        if let Some(r) = &self.report {
            items.extend(self.view(cx, r));
        }
        items
    }

    fn act(&mut self, _act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        self.status.clear();
        self.report = None;
        let request = json!({"verify": page.checked("verify"), "remotes": page.checked("remotes")});
        match cx.start("report", request) {
            Ok(job) => page.job = Some(Running { id: "report", job }),
            Err(e) => self.status.error(cx, &e),
        }
        Step::Stay
    }

    fn done(
        &mut self,
        _id: &'static str,
        result: apeiron::Result<Value>,
        _page: &mut Page,
        cx: &mut Ctx,
    ) -> Step {
        match result {
            Ok(r) => self.report = Some(r),
            Err(e) => self.status.error(cx, &e),
        }
        Step::Stay
    }
}

#[derive(Default)]
pub(crate) struct Retention {
    rules: Vec<Value>,
    status: Status,
}

impl Screen for Retention {
    fn nav(&self) -> Nav {
        Nav::Retention
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        self.rules = cx
            .call("retention", Value::Null)?
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("retention_heading")),
            muted(cx.t("retention_intro")),
        ];
        if self.rules.is_empty() {
            items.push(Item::Text(cx.t("retention_none"), Tone::Plain));
        } else {
            items.push(Item::Row {
                cells: vec![
                    cx.t("retention_refs"),
                    cx.t("retention_days"),
                    cx.t("retention_added"),
                ],
                tone: Tone::Muted,
                buttons: Vec::new(),
            });
        }
        for r in &self.rules {
            items.push(row(
                vec![
                    text(r, "refs"),
                    cx.tw("report_days", &[("days", num(r, "days").to_string())]),
                    format::seconds(r["added"].as_i64().unwrap_or(0)),
                ],
                Vec::new(),
            ));
        }
        items.extend([
            Item::Heading(cx.t("retention_add")),
            Item::Input(
                input("refs", cx.t("retention_refs"))
                    .hint(cx.t("retention_refs_hint"))
                    .submit(act("add")),
            ),
            Item::Input(
                input("days", cx.t("retention_days"))
                    .hint(cx.t("retention_days_hint"))
                    .submit(act("add")),
            ),
            Item::Buttons(vec![button('a', cx.t("retention_add"), act("add"))]),
        ]);
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        self.status.clear();
        let refs = page.value("refs").trim().to_owned();
        let days = page.value("days").trim().to_owned();
        match act.id {
            "add" => {
                let question = cx.tw("retention_ask", &[("refs", refs), ("days", days)]);
                page.ask(question, crate::page::act("sure"));
                Step::Stay
            }
            "sure" => {
                let Ok(days) = days.parse::<u32>() else {
                    self.status.set(
                        Tone::Bad,
                        format!("{}: {}", cx.t("error"), cx.t("retention_days_hint")),
                    );
                    return Step::Stay;
                };
                match cx.call("add_retention", json!({"refs": refs, "days": days})) {
                    Ok(_) => Step::Reload,
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
