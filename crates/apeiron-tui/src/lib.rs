//! The terminal interface of Apeiron (`docs/TUI.md`): the screens of the
//! desktop interface drawn in a terminal. Every screen calls the operations
//! by name of the public API, as the pages do.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod format;
mod page;
mod screens;
mod words;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use apeiron::json::JsonJob;
use apeiron::{Error, ErrorKind, Repository};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use page::{Act, Outcome, Page};
pub use words::system_lang;

/// What the terminal interface starts with.
pub struct Options {
    /// A repository open from the start.
    pub repo: Option<Repository>,
    /// Where the last repository opened is remembered, the same file as the
    /// desktop interface's; `None` remembers nothing.
    pub memory: Option<PathBuf>,
    /// The background service's jobs file and state; `None` hides the
    /// schedule.
    pub schedule: Option<apeiron_daemon::Paths>,
    /// The plugin folder; the user's when `None`.
    pub plugins: Option<PathBuf>,
    /// `tr` or `en`.
    pub lang: String,
}

impl Options {
    /// The usual places of the user, as the desktop interface uses them.
    pub fn for_user() -> Self {
        Options {
            repo: None,
            memory: dirs::config_dir().map(|d| d.join("apeiron").join("desktop.json")),
            schedule: apeiron_daemon::Paths::system().ok(),
            plugins: None,
            lang: system_lang().to_owned(),
        }
    }
}

/// What the screens share: the open repository and the words.
pub(crate) struct Ctx {
    pub w: words::Words,
    pub repo: Option<Repository>,
    pub memory: Option<PathBuf>,
    pub schedule: Option<apeiron_daemon::Paths>,
    pub plugins: Option<PathBuf>,
}

#[derive(Default, Serialize, Deserialize)]
struct Memory {
    last: Option<PathBuf>,
}

impl Ctx {
    pub fn t(&self, key: &str) -> String {
        self.w.get(key)
    }

    pub fn tw(&self, key: &str, values: &[(&str, String)]) -> String {
        self.w.with(key, values)
    }

    /// Runs a short operation, on the open repository when there is one.
    pub fn call(&self, operation: &str, request: Value) -> apeiron::Result<Value> {
        match &self.repo {
            Some(repo) => apeiron::json::call(repo, operation, request),
            None => apeiron::json::call_global(operation, request),
        }
    }

    /// Starts a long operation on the open repository.
    pub fn start(&self, operation: &str, request: Value) -> apeiron::Result<Box<dyn JsonJob>> {
        let repo = self
            .repo
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "no repository is open"))?;
        apeiron::json::start(repo, operation, request)
    }

    /// An error as the pages show it.
    pub fn error(&self, e: &Error) -> String {
        if e.kind == ErrorKind::Cancelled {
            self.t("cancelled")
        } else {
            format!("{}: {}", self.t("error"), e.message)
        }
    }

    pub fn last(&self) -> Option<PathBuf> {
        let file = self.memory.as_ref()?;
        let bytes = std::fs::read(file).ok()?;
        serde_json::from_slice::<Memory>(&bytes).ok()?.last
    }

    /// Remembers the repository opened last; failing is not worth an error.
    pub fn remember(&self, path: &std::path::Path) {
        if let Some(file) = &self.memory {
            let memory = Memory {
                last: Some(path.to_path_buf()),
            };
            if let (Some(dir), Ok(data)) = (file.parent(), serde_json::to_vec(&memory)) {
                let _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(file, data));
            }
        }
    }
}

/// The screens of the menu, in the order of the pages' menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nav {
    Overview,
    Backup,
    Verify,
    Report,
    Retention,
    Sync,
    Keys,
    Plugins,
    Schedule,
    Reviews,
    Close,
    /// Not in the menu: the start screen.
    None,
}

impl Nav {
    fn key(self) -> &'static str {
        match self {
            Nav::Overview => "nav_overview",
            Nav::Backup => "nav_backup",
            Nav::Verify => "nav_verify",
            Nav::Report => "nav_report",
            Nav::Retention => "nav_retention",
            Nav::Sync => "nav_sync",
            Nav::Keys => "nav_keys",
            Nav::Plugins => "nav_plugins",
            Nav::Schedule => "nav_schedule",
            Nav::Reviews => "nav_reviews",
            Nav::Close | Nav::None => "nav_close",
        }
    }
}

/// What a screen asks for after a key or a finished job.
pub(crate) enum Step {
    Stay,
    /// Loads the screen's data again and redraws it.
    Reload,
    Push(Box<dyn Screen>),
    /// A repository was opened: the overview.
    Opened,
    /// Opens `text` in the user's editor; the screen gets it back edited.
    Edit(String),
}

/// One screen: what it loads, what it shows and what its buttons do.
pub(crate) trait Screen {
    fn nav(&self) -> Nav;

    /// Reads what the screen shows; an error is shown instead of it.
    fn load(&mut self, _cx: &Ctx) -> apeiron::Result<()> {
        Ok(())
    }

    /// Sets the first values of its fields, once.
    fn init(&mut self, _page: &mut Page, _cx: &Ctx) {}

    fn build(&self, cx: &Ctx, page: &Page) -> Vec<page::Item>;

    fn act(&mut self, act: Act, page: &mut Page, cx: &mut Ctx) -> Step;

    /// A long operation it started has ended.
    fn done(
        &mut self,
        _id: &'static str,
        _result: apeiron::Result<Value>,
        _page: &mut Page,
        _cx: &mut Ctx,
    ) -> Step {
        Step::Stay
    }

    /// A field's value changed.
    fn changed(&mut self, _id: &'static str, _page: &mut Page) {}

    /// The text it asked to edit, after the editor closed.
    fn edited(&mut self, _text: io::Result<String>, _page: &mut Page, _cx: &mut Ctx) -> Step {
        Step::Stay
    }
}

struct Shown {
    screen: Box<dyn Screen>,
    page: Page,
    /// Why its data could not be read.
    failed: Option<String>,
}

/// The terminal interface's state; drawn and driven by keys.
pub struct App {
    cx: Ctx,
    stack: Vec<Shown>,
    /// The chosen menu item while the menu has the keys.
    menu: Option<usize>,
    quit: bool,
    /// Text waiting to be edited in the user's editor.
    edit: Option<String>,
}

impl App {
    pub fn new(options: Options) -> Self {
        let opened = options.repo.is_some();
        let mut app = App {
            cx: Ctx {
                w: words::Words::new(&options.lang),
                repo: options.repo,
                memory: options.memory,
                schedule: options.schedule,
                plugins: options.plugins,
            },
            stack: Vec::new(),
            menu: None,
            quit: false,
            edit: None,
        };
        if opened {
            app.root(Box::new(screens::Overview::default()));
        } else {
            app.root(Box::new(screens::Start::default()));
        }
        app
    }

    /// Whether the program should end.
    pub fn done(&self) -> bool {
        self.quit
    }

    fn navs(&self) -> Vec<Nav> {
        let mut navs = vec![
            Nav::Overview,
            Nav::Backup,
            Nav::Verify,
            Nav::Report,
            Nav::Retention,
            Nav::Sync,
            Nav::Keys,
            Nav::Plugins,
        ];
        if self.cx.schedule.is_some() {
            navs.push(Nav::Schedule);
        }
        navs.extend([Nav::Reviews, Nav::Close]);
        navs
    }

    fn shown(&mut self, screen: Box<dyn Screen>) -> Shown {
        let mut shown = Shown {
            screen,
            page: Page::default(),
            failed: None,
        };
        shown.failed = shown.screen.load(&self.cx).err().map(|e| self.cx.error(&e));
        shown.screen.init(&mut shown.page, &self.cx);
        rebuild(&mut shown, &self.cx);
        shown.page.top();
        shown
    }

    fn root(&mut self, screen: Box<dyn Screen>) {
        for s in &mut self.stack {
            s.page.stop();
        }
        self.stack.clear();
        let shown = self.shown(screen);
        self.stack.push(shown);
    }

    fn open_nav(&mut self, nav: Nav) {
        self.menu = None;
        let screen: Box<dyn Screen> = match nav {
            Nav::Overview => Box::new(screens::Overview::default()),
            Nav::Backup => Box::new(screens::Backup::new("", "")),
            Nav::Verify => Box::new(screens::Verify::default()),
            Nav::Report => Box::new(screens::Report::default()),
            Nav::Retention => Box::new(screens::Retention::default()),
            Nav::Sync => Box::new(screens::Sync::default()),
            Nav::Keys => Box::new(screens::Keys::default()),
            Nav::Plugins => Box::new(screens::Plugins::default()),
            Nav::Schedule => Box::new(screens::Schedule::default()),
            Nav::Reviews => Box::new(screens::Reviews::default()),
            Nav::Close | Nav::None => {
                self.cx.repo = None;
                Box::new(screens::Start::default())
            }
        };
        self.root(screen);
    }

    fn step(&mut self, step: Step) {
        match step {
            Step::Stay => {}
            Step::Reload => {
                if let Some(top) = self.stack.last_mut() {
                    top.failed = top.screen.load(&self.cx).err().map(|e| self.cx.error(&e));
                }
            }
            Step::Push(screen) => {
                let shown = self.shown(screen);
                self.stack.push(shown);
            }
            Step::Opened => self.open_nav(Nav::Overview),
            Step::Edit(text) => self.edit = Some(text),
        }
        if let Some(top) = self.stack.last_mut() {
            rebuild(top, &self.cx);
        }
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
            if let Some(top) = self.stack.last_mut() {
                top.failed = top.screen.load(&self.cx).err().map(|e| self.cx.error(&e));
                rebuild(top, &self.cx);
            }
        } else if self.cx.repo.is_some() {
            let nav = self.stack.last().map_or(Nav::Overview, |s| s.screen.nav());
            self.menu = Some(self.navs().iter().position(|n| *n == nav).unwrap_or(0));
        }
    }

    /// Handles one key press.
    pub fn key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            for s in &mut self.stack {
                s.page.stop();
            }
            self.quit = true;
            return;
        }
        if let Some(chosen) = self.menu {
            let navs = self.navs();
            match key.code {
                KeyCode::Up | KeyCode::BackTab => self.menu = Some(chosen.saturating_sub(1)),
                KeyCode::Down | KeyCode::Tab => self.menu = Some((chosen + 1).min(navs.len() - 1)),
                KeyCode::Enter | KeyCode::Right => self.open_nav(navs[chosen]),
                KeyCode::Esc => self.menu = None,
                KeyCode::Char('q') => self.quit = true,
                _ => {}
            }
            return;
        }
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        match top.page.key(key) {
            Outcome::Nothing => {}
            Outcome::Back => self.back(),
            Outcome::Changed(id) => {
                top.screen.changed(id, &mut top.page);
                rebuild(top, &self.cx);
            }
            Outcome::Act(act) => {
                let step = top.screen.act(act, &mut top.page, &mut self.cx);
                self.step(step);
            }
        }
    }

    /// Collects a finished job; called between keys.
    pub fn tick(&mut self) {
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        if let Some((id, result)) = top.page.finished() {
            let step = top.screen.done(id, result, &mut top.page, &mut self.cx);
            self.step(step);
        }
    }

    /// Text a screen wants edited; the caller runs the editor.
    pub fn take_edit(&mut self) -> Option<String> {
        self.edit.take()
    }

    /// The edited text, or why the editor did not run.
    pub fn edited(&mut self, text: io::Result<String>) {
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        let step = top.screen.edited(text, &mut top.page, &mut self.cx);
        self.step(step);
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let [main, help] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let content = if self.cx.repo.is_some() {
            let [menu, content] =
                Layout::horizontal([Constraint::Length(24), Constraint::Min(10)]).areas(main);
            let nav = self.stack.last().map_or(Nav::None, |s| s.screen.nav());
            let lines: Vec<Line> = self
                .navs()
                .iter()
                .enumerate()
                .map(|(n, item)| {
                    let label = format!(" {}", self.cx.t(item.key()));
                    let mut style = Style::default();
                    if *item == nav {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if self.menu == Some(n) {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    Line::styled(label, style)
                })
                .collect();
            let block = Block::default()
                .borders(Borders::RIGHT)
                .title(format!(" {} ", self.cx.t("title")));
            f.render_widget(Paragraph::new(lines).block(block), menu);
            content
        } else {
            main
        };
        let inner = content.inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 0,
        });
        let w = &self.cx.w;
        if let Some(top) = self.stack.last_mut() {
            if let Some(failed) = &top.failed {
                let mut lines = vec![Line::styled(
                    failed.clone(),
                    Style::default().fg(ratatui::style::Color::Red),
                )];
                lines.push(Line::default());
                lines.push(Line::raw(w.get("tui_back")));
                f.render_widget(
                    Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
                    inner,
                );
            } else {
                top.page.draw(f, inner, w);
            }
        }
        let keys = if self.menu.is_some() {
            w.get("tui_keys_menu")
        } else {
            w.get("tui_keys")
        };
        let path = self
            .cx
            .repo
            .as_ref()
            .map(|r| format!("  ·  {}", r.path().display()))
            .unwrap_or_default();
        f.render_widget(
            Paragraph::new(Line::styled(
                format!("{keys}{path}"),
                Style::default().add_modifier(Modifier::DIM),
            )),
            help,
        );
    }
}

fn rebuild(shown: &mut Shown, cx: &Ctx) {
    if shown.failed.is_none() {
        let items = shown.screen.build(cx, &shown.page);
        shown.page.show(items);
    }
}

/// Runs the terminal interface until the user leaves it.
pub fn run(options: Options) -> io::Result<()> {
    let mut app = App::new(options);
    let mut terminal = ratatui::init();
    let result = drive(&mut app, &mut terminal);
    ratatui::restore();
    result
}

fn drive(app: &mut App, terminal: &mut DefaultTerminal) -> io::Result<()> {
    while !app.done() {
        terminal.draw(|f| app.draw(f))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            app.key(key);
        }
        app.tick();
        if let Some(text) = app.take_edit() {
            ratatui::restore();
            let edited = edit(&text);
            *terminal = ratatui::init();
            terminal.clear()?;
            app.edited(edited);
        }
    }
    Ok(())
}

/// Opens `text` in `VISUAL`, `EDITOR` or a usual editor, and returns it as
/// the user saved it.
fn edit(text: &str) -> io::Result<String> {
    let file = std::env::temp_dir().join(format!("apeiron-jobs-{}.toml", std::process::id()));
    std::fs::write(&file, text)?;
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".to_owned()
            } else {
                "vi".to_owned()
            }
        });
    // The editor may come with options, such as `code --wait`.
    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(&file)
        .status();
    let read = status.and_then(|s| {
        if s.success() {
            std::fs::read_to_string(&file)
        } else {
            Err(io::Error::other(format!("{program} ended with {s}")))
        }
    });
    let _ = std::fs::remove_file(&file);
    read
}
