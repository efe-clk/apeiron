//! Opening or creating a repository, and a new recovery key.

use std::path::PathBuf;

use apeiron::{ErrorKind, Repository};

use super::Status;
use crate::page::{self, Act, Item, Page, Tone, act, button, input, muted, text};
use crate::{Ctx, Nav, Screen, Step};

#[derive(Default)]
pub(crate) struct Start {
    last: Option<PathBuf>,
    /// The repository to open is locked: its password is asked.
    locked: bool,
    open_status: Status,
    new_status: Status,
}

impl Screen for Start {
    fn nav(&self) -> Nav {
        Nav::None
    }

    fn init(&mut self, page: &mut Page, cx: &Ctx) {
        self.last = cx.last();
        if let Some(last) = &self.last {
            page.set("open_path", last.display().to_string());
        }
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        let mut items = vec![
            Item::Heading(cx.t("start_heading")),
            muted(cx.t("start_intro")),
            Item::Heading(cx.t("open_heading")),
        ];
        if let Some(last) = &self.last {
            items.push(muted(format!("{}: {}", cx.t("last_used"), last.display())));
        }
        items.push(Item::Input(
            input("open_path", cx.t("path"))
                .folder()
                .submit(act("open")),
        ));
        if self.locked {
            items.push(Item::Input(
                input("open_password", cx.t("password"))
                    .secret()
                    .submit(act("open")),
            ));
        }
        items.extend(self.open_status.items());
        items.push(Item::Buttons(vec![button(
            'o',
            cx.t("open_button"),
            act("open"),
        )]));
        items.extend([
            Item::Heading(cx.t("new_heading")),
            muted(cx.t("new_intro")),
            Item::Input(input("new_path", cx.t("path")).folder().submit(act("new"))),
            Item::Input(
                input("new_password", cx.t("password_optional"))
                    .secret()
                    .submit(act("new")),
            ),
            Item::Input(
                input("new_again", cx.t("password_again"))
                    .secret()
                    .submit(act("new")),
            ),
        ]);
        items.extend(self.new_status.items());
        items.push(Item::Buttons(vec![button(
            'n',
            cx.t("new_button"),
            act("new"),
        )]));
        items
    }

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step {
        match act.id {
            "open" => {
                self.open_status.clear();
                let path = PathBuf::from(page.value("open_path").trim());
                let password = page.value("open_password");
                let opened = if password.is_empty() {
                    Repository::open(&path)
                } else {
                    Repository::unlock(&path, &password)
                };
                match opened {
                    Ok(repo) => {
                        cx.remember(&path);
                        cx.repo = Some(repo);
                        Step::Opened
                    }
                    Err(e) if e.kind == ErrorKind::Key && password.is_empty() => {
                        self.locked = true;
                        self.open_status.set(Tone::Plain, cx.t("password_needed"));
                        let items = self.build(cx, page);
                        page.show(items);
                        page.focus_on("open_password");
                        Step::Stay
                    }
                    Err(e) => {
                        self.open_status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            "new" => {
                self.new_status.clear();
                let password = page.value("new_password");
                if password != page.value("new_again") {
                    self.new_status.set(Tone::Bad, cx.t("passwords_differ"));
                    return Step::Stay;
                }
                let path = PathBuf::from(page.value("new_path").trim());
                let made = if password.is_empty() {
                    Repository::init(&path).map(|r| (r, None))
                } else {
                    Repository::init_encrypted(&path, &password).map(|(r, k)| (r, Some(k)))
                };
                match made {
                    Ok((repo, key)) => {
                        cx.remember(&path);
                        cx.repo = Some(repo);
                        match key {
                            Some(key) => Step::Push(Box::new(Recovery { key })),
                            None => Step::Opened,
                        }
                    }
                    Err(e) => {
                        self.new_status.error(cx, &e);
                        Step::Stay
                    }
                }
            }
            _ => Step::Stay,
        }
    }
}

/// A new repository's recovery key, shown once.
struct Recovery {
    key: String,
}

impl Screen for Recovery {
    fn nav(&self) -> Nav {
        Nav::None
    }

    fn build(&self, cx: &Ctx, _page: &Page) -> Vec<Item> {
        vec![
            Item::Heading(cx.t("recovery_heading")),
            text(cx.t("recovery_text")),
            Item::Gap,
            Item::Text(self.key.clone(), Tone::Key),
            Item::Gap,
            Item::Buttons(vec![button('d', cx.t("recovery_done"), page::act("done"))]),
        ]
    }

    fn act(&mut self, _act: Act, _page: &mut Page, _cx: &mut Ctx) -> Step {
        Step::Opened
    }
}
