//! The background service's jobs (DAEMON.md §3): their last and next runs,
//! and the jobs file, edited in the user's editor.

use std::io;

use apeiron_daemon::JobStatus;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use super::Status;
use crate::page::{Act, Item, Page, Tone, act, button, muted, row};
use crate::{Ctx, Nav, Screen, Step};

#[derive(Default)]
pub(crate) struct Schedule {
    file: String,
    text: String,
    jobs: Vec<JobStatus>,
    /// Why the jobs file cannot be read as jobs.
    invalid: Option<String>,
    /// What the user saved last that was not valid, to edit again.
    draft: Option<String>,
    status: Status,
}

fn local(t: Timestamp) -> String {
    t.to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}

impl Screen for Schedule {
    fn nav(&self) -> Nav {
        Nav::Schedule
    }

    fn load(&mut self, cx: &Ctx) -> apeiron::Result<()> {
        let Some(paths) = &cx.schedule else {
            return Ok(());
        };
        self.file = paths.jobs.display().to_string();
        self.text = match std::fs::read_to_string(&paths.jobs) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return Err(apeiron::Error::new(
                    apeiron::ErrorKind::Io,
                    format!("{}: {e}", paths.jobs.display()),
                ));
            }
        };
        match apeiron_daemon::status(paths) {
            Ok(jobs) => {
                self.jobs = jobs;
                self.invalid = None;
            }
            Err(e) => {
                self.jobs.clear();
                self.invalid = Some(e.to_string());
            }
        }
        Ok(())
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("schedule_heading")),
            muted(cx.t("schedule_intro")),
        ];
        if let Some(invalid) = &self.invalid {
            items.push(Item::Text(
                format!("{} {invalid}", cx.t("schedule_invalid")),
                Tone::Bad,
            ));
        } else if self.jobs.is_empty() {
            items.push(Item::Text(cx.t("schedule_none"), Tone::Plain));
        } else {
            items.push(Item::Row {
                cells: [
                    "schedule_name",
                    "schedule_do",
                    "schedule_last",
                    "schedule_next",
                ]
                .iter()
                .map(|k| cx.t(k))
                .collect(),
                tone: Tone::Muted,
                buttons: Vec::new(),
            });
        }
        for j in &self.jobs {
            let last = match &j.last {
                Some(l) => {
                    let result = serde_json::to_value(l.result)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    let mut s = format!(
                        "{} · {}",
                        local(l.last_end),
                        cx.t(&format!("schedule_result_{result}"))
                    );
                    if !l.message.is_empty() {
                        s = format!("{s} · {}", l.message);
                    }
                    s
                }
                None => cx.t("schedule_never"),
            };
            items.push(row(
                vec![
                    j.name.clone(),
                    cx.t(&format!("schedule_do_{}", j.action)),
                    last,
                    local(j.next_run),
                ],
                Vec::new(),
            ));
        }
        items.extend([
            Item::Gap,
            muted(format!(
                "{}: {}",
                cx.t("schedule_file"),
                cx.tw("schedule_file_hint", &[("file", self.file.clone())])
            )),
            Item::Buttons(vec![button('e', cx.t("tui_schedule_edit"), act("edit"))]),
        ]);
        items.extend(self.status.items());
        items
    }

    fn act(&mut self, _act: Act, _page: &mut Page, cx: &mut Ctx) -> Step {
        self.status.clear();
        let text = match (&self.draft, self.text.is_empty()) {
            (Some(draft), _) => draft.clone(),
            (None, false) => self.text.clone(),
            (None, true) => {
                let repo = cx
                    .repo
                    .as_ref()
                    .map(|r| r.path().display().to_string())
                    .unwrap_or_default();
                cx.tw("schedule_example", &[("repo", repo)])
            }
        };
        Step::Edit(text)
    }

    fn edited(&mut self, text: io::Result<String>, _page: &mut Page, cx: &mut Ctx) -> Step {
        let text = match text {
            Ok(text) => text,
            Err(e) => {
                self.status
                    .set(Tone::Bad, format!("{}: {e}", cx.t("error")));
                return Step::Stay;
            }
        };
        let Some(paths) = &cx.schedule else {
            return Step::Stay;
        };
        if text == self.text && self.draft.is_none() {
            return Step::Stay;
        }
        match apeiron_daemon::save_scoped(&paths.jobs, &text, paths.scope.as_ref()) {
            Ok(jobs) => {
                self.draft = None;
                self.status.set(
                    Tone::Good,
                    cx.tw("schedule_saved", &[("count", jobs.len().to_string())]),
                );
                Step::Reload
            }
            Err(e) => {
                self.draft = Some(text);
                self.status
                    .set(Tone::Bad, format!("{}: {e}", cx.t("error")));
                self.status.push(Tone::Muted, cx.t("tui_schedule_draft"));
                Step::Stay
            }
        }
    }
}
