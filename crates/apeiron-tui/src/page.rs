//! What a screen shows, as a list of items, and how keys move through it:
//! fields, rows with buttons, the progress of a job, a question and the
//! folder browser (TUI.md §3).

use std::collections::HashMap;
use std::path::PathBuf;

use apeiron::json::JsonJob;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::format;
use crate::words::Words;

/// How a text is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Muted,
    Good,
    Bad,
    /// A recovery key: shown so it stands out.
    Key,
}

impl Tone {
    fn style(self) -> Style {
        match self {
            Tone::Plain => Style::default(),
            Tone::Muted => Style::default().add_modifier(Modifier::DIM),
            Tone::Good => Style::default().fg(Color::Green),
            Tone::Bad => Style::default().fg(Color::Red),
            Tone::Key => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        }
    }
}

/// What a button or a field asks the screen to do; `arg` is usually the
/// number of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Act {
    pub id: &'static str,
    pub arg: usize,
}

pub fn act(id: &'static str) -> Act {
    Act { id, arg: 0 }
}

pub fn act_on(id: &'static str, arg: usize) -> Act {
    Act { id, arg }
}

/// A button: its key, shown as `[k]`, and what it does.
#[derive(Clone, Debug)]
pub struct Button {
    pub key: char,
    pub label: String,
    pub act: Act,
}

pub fn button(key: char, label: impl Into<String>, act: Act) -> Button {
    Button {
        key,
        label: label.into(),
        act,
    }
}

/// A text field.
#[derive(Clone, Debug)]
pub struct Input {
    pub id: &'static str,
    pub label: String,
    pub hint: Option<String>,
    /// Shown as dots: a password.
    pub secret: bool,
    /// `F2` opens the folder browser.
    pub folder: bool,
    /// What `Enter` does.
    pub submit: Option<Act>,
}

pub fn input(id: &'static str, label: impl Into<String>) -> Input {
    Input {
        id,
        label: label.into(),
        hint: None,
        secret: false,
        folder: false,
        submit: None,
    }
}

impl Input {
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn folder(mut self) -> Self {
        self.folder = true;
        self
    }

    pub fn submit(mut self, act: Act) -> Self {
        self.submit = Some(act);
        self
    }
}

/// One part of a screen, top to bottom.
#[derive(Clone, Debug)]
pub enum Item {
    Heading(String),
    Text(String, Tone),
    /// Lines shown as they are, such as a diff.
    Pre(Vec<Line<'static>>),
    Input(Input),
    Check {
        id: &'static str,
        label: String,
    },
    /// One of several values, the first shown text, the second sent.
    Select {
        id: &'static str,
        label: String,
        options: Vec<(String, String)>,
    },
    /// A row of a table; rows next to each other share their column widths.
    /// It can be chosen when it has buttons.
    Row {
        cells: Vec<String>,
        tone: Tone,
        buttons: Vec<Button>,
    },
    Buttons(Vec<Button>),
    Gap,
}

pub fn text(s: impl Into<String>) -> Item {
    Item::Text(s.into(), Tone::Plain)
}

pub fn muted(s: impl Into<String>) -> Item {
    Item::Text(s.into(), Tone::Muted)
}

pub fn row(cells: Vec<String>, buttons: Vec<Button>) -> Item {
    Item::Row {
        cells,
        tone: Tone::Plain,
        buttons,
    }
}

impl Item {
    fn focusable(&self) -> bool {
        match self {
            Item::Input(_) | Item::Check { .. } | Item::Select { .. } => true,
            Item::Row { buttons, .. } | Item::Buttons(buttons) => !buttons.is_empty(),
            _ => false,
        }
    }
}

/// A long operation running, with its progress drawn.
pub struct Running {
    pub id: &'static str,
    pub job: Box<dyn JsonJob>,
}

/// The folder browser over a field.
pub struct Picker {
    field: &'static str,
    path: PathBuf,
    folders: Vec<String>,
    error: Option<String>,
    chosen: usize,
}

impl Picker {
    fn open(field: &'static str, start: &str) -> Self {
        let path = Some(PathBuf::from(start))
            .filter(|p| !start.is_empty() && p.is_dir())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        let mut picker = Picker {
            field,
            path,
            folders: Vec::new(),
            error: None,
            chosen: 0,
        };
        picker.list();
        picker
    }

    /// The folders in `path`, without hidden ones, as the pages list them.
    fn list(&mut self) {
        self.chosen = 0;
        self.error = None;
        match std::fs::read_dir(&self.path) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| !n.starts_with('.'))
                    .collect();
                names.sort_by_key(|n| n.to_lowercase());
                self.folders = names;
            }
            Err(e) => {
                self.folders.clear();
                self.error = Some(format!("{}: {e}", self.path.display()));
            }
        }
    }

    /// Rows: choose this folder, up, then the folders.
    fn rows(&self) -> usize {
        2 + self.folders.len()
    }

    fn enter(&mut self, path: PathBuf) {
        self.path = path;
        self.list();
    }

    /// `Some(path)` when this folder was chosen.
    fn key(&mut self, key: KeyEvent) -> Option<Option<PathBuf>> {
        match key.code {
            KeyCode::Esc => return Some(None),
            KeyCode::Up => self.chosen = self.chosen.saturating_sub(1),
            KeyCode::Down => self.chosen = (self.chosen + 1).min(self.rows() - 1),
            KeyCode::PageUp => self.chosen = self.chosen.saturating_sub(10),
            KeyCode::PageDown => self.chosen = (self.chosen + 10).min(self.rows() - 1),
            KeyCode::Backspace | KeyCode::Left => {
                if let Some(parent) = self.path.parent() {
                    self.enter(parent.to_path_buf());
                }
            }
            KeyCode::Enter | KeyCode::Right => match self.chosen {
                0 => return Some(Some(self.path.clone())),
                1 => {
                    if let Some(parent) = self.path.parent() {
                        self.enter(parent.to_path_buf());
                    }
                }
                n => {
                    let name = self.folders[n - 2].clone();
                    self.enter(self.path.join(name));
                }
            },
            _ => {}
        }
        None
    }

    fn draw(&self, f: &mut Frame, area: Rect, w: &Words) {
        let width = area.width.saturating_sub(8).clamp(20, 80);
        let height = area.height.saturating_sub(4).clamp(6, 30);
        let rect = centered(area, width, height);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", w.get("choose_folder")));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let mut lines = vec![Line::styled(
            self.path.display().to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        if let Some(e) = &self.error {
            lines.push(Line::styled(e.clone(), Tone::Bad.style()));
        }
        let first = lines.len();
        let mut rows = vec![
            format!("✓ {}", w.get("choose_this")),
            format!("↑ {}", w.get("up")),
        ];
        rows.extend(self.folders.iter().map(|n| format!("📁 {n}")));
        let room = (inner.height as usize).saturating_sub(first).max(1);
        let top = self.chosen.saturating_sub(room - 1);
        for (i, r) in rows.iter().enumerate().skip(top).take(room) {
            let style = if i == self.chosen {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(r.clone(), style));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// What a key did to a page.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Act(Act),
    /// A field's value changed.
    Changed(&'static str),
    Back,
}

/// The state of a screen as shown: its items and what was typed and chosen.
#[derive(Default)]
pub struct Page {
    pub items: Vec<Item>,
    values: HashMap<&'static str, String>,
    checks: HashMap<&'static str, bool>,
    choices: HashMap<&'static str, usize>,
    focus: usize,
    /// The chosen button of a button row.
    button: usize,
    scroll: usize,
    /// Scroll to the focused item at the next draw.
    follow: bool,
    pub job: Option<Running>,
    confirm: Option<(String, Act)>,
    picker: Option<Picker>,
}

impl Page {
    pub fn value(&self, id: &str) -> String {
        self.values.get(id).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, id: &'static str, value: impl Into<String>) {
        self.values.insert(id, value.into());
    }

    pub fn checked(&self, id: &str) -> bool {
        self.checks.get(id).copied().unwrap_or(false)
    }

    /// The value sent for the chosen option of a select item.
    pub fn choice(&self, id: &str) -> String {
        let n = self.choices.get(id).copied().unwrap_or(0);
        self.items
            .iter()
            .find_map(|i| match i {
                Item::Select { id: i, options, .. } if *i == id => {
                    options.get(n).map(|o| o.1.clone())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Chooses the option sending `value`, when there is one.
    pub fn choose(&mut self, id: &'static str, value: &str) {
        let at = self.items.iter().find_map(|i| match i {
            Item::Select { id: i, options, .. } if *i == id => {
                options.iter().position(|o| o.1 == value)
            }
            _ => None,
        });
        if let Some(n) = at {
            self.choices.insert(id, n);
        }
    }

    /// Asks `question`; `y` does `act`.
    pub fn ask(&mut self, question: String, act: Act) {
        self.confirm = Some((question, act));
    }

    /// Replaces the items, keeping the focus on the same field, row or
    /// buttons where it can.
    pub fn show(&mut self, items: Vec<Item>) {
        let was = self.items.get(self.focus).and_then(identity);
        self.items = items;
        if let Some(n) = was.and_then(|was| {
            self.items
                .iter()
                .position(|i| identity(i).is_some_and(|id| id == was))
        }) && n != self.focus
        {
            self.focus = n;
            self.follow = true;
        }
        if !self.items.get(self.focus).is_some_and(Item::focusable) {
            self.focus = self.next_focus(self.focus, true).unwrap_or(0);
            self.follow = true;
        }
        if let Some(Item::Buttons(b)) = self.items.get(self.focus) {
            self.button = self.button.min(b.len().saturating_sub(1));
        }
    }

    /// Moves the focus to the first item.
    pub fn top(&mut self) {
        self.focus = self.next_focus(0, true).unwrap_or(0);
        self.button = 0;
        self.scroll = 0;
    }

    /// Moves the focus to the field `id`.
    pub fn focus_on(&mut self, id: &str) {
        if let Some(n) = self.items.iter().position(|i| match i {
            Item::Input(input) => input.id == id,
            Item::Check { id: i, .. } | Item::Select { id: i, .. } => *i == id,
            _ => false,
        }) {
            self.focus = n;
            self.follow = true;
        }
    }

    /// The first focusable item from `from` on, forwards or backwards.
    fn next_focus(&self, from: usize, forwards: bool) -> Option<usize> {
        if forwards {
            (from..self.items.len()).find(|&i| self.items[i].focusable())
        } else {
            (0..=from.min(self.items.len().saturating_sub(1)))
                .rev()
                .find(|&i| self.items[i].focusable())
        }
    }

    fn move_focus(&mut self, forwards: bool) {
        let next = if forwards {
            self.next_focus(self.focus + 1, true)
        } else if self.focus == 0 {
            None
        } else {
            self.next_focus(self.focus - 1, false)
        };
        match next {
            Some(n) if self.items[self.focus].focusable() || n != self.focus => {
                self.focus = n;
                self.button = 0;
                self.follow = true;
            }
            _ if forwards => self.scroll += 1,
            _ => self.scroll = self.scroll.saturating_sub(1),
        }
    }

    /// The button with `key` on the focused row, else on any button row.
    fn button_for(&self, key: char) -> Option<Act> {
        let find = |b: &[Button]| b.iter().find(|b| b.key == key).map(|b| b.act);
        if let Some(Item::Row { buttons, .. }) = self.items.get(self.focus)
            && let Some(a) = find(buttons)
        {
            return Some(a);
        }
        self.items.iter().find_map(|i| match i {
            Item::Buttons(b) => find(b),
            _ => None,
        })
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if let Some((_, act)) = self.confirm.take() {
            return match key.code {
                KeyCode::Char('y' | 'Y' | 'e' | 'E') => Outcome::Act(act),
                _ => Outcome::Nothing,
            };
        }
        if let Some(picker) = &mut self.picker {
            if let Some(chosen) = picker.key(key) {
                let field = picker.field;
                self.picker = None;
                if let Some(path) = chosen {
                    self.set(field, path.display().to_string());
                    return Outcome::Changed(field);
                }
            }
            return Outcome::Nothing;
        }
        if let Some(running) = &self.job {
            if key.code == KeyCode::Esc {
                running.job.cancel();
            }
            return Outcome::Nothing;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Outcome::Back,
            KeyCode::Tab | KeyCode::Down => {
                self.move_focus(true);
                return Outcome::Nothing;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.move_focus(false);
                return Outcome::Nothing;
            }
            KeyCode::PageDown => {
                self.scroll += 10;
                return Outcome::Nothing;
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(10);
                return Outcome::Nothing;
            }
            _ => {}
        }
        let Some(item) = self.items.get(self.focus).filter(|i| i.focusable()) else {
            return match key.code {
                KeyCode::Char(c) => self.button_for(c).map_or(Outcome::Nothing, Outcome::Act),
                _ => Outcome::Nothing,
            };
        };
        match item.clone() {
            Item::Input(input) => match key.code {
                KeyCode::Enter => input.submit.map_or(Outcome::Nothing, Outcome::Act),
                KeyCode::F(2) if input.folder => {
                    self.picker = Some(Picker::open(input.id, &self.value(input.id)));
                    Outcome::Nothing
                }
                KeyCode::Backspace => {
                    self.values.entry(input.id).or_default().pop();
                    Outcome::Changed(input.id)
                }
                KeyCode::Char('u') if ctrl => {
                    self.values.insert(input.id, String::new());
                    Outcome::Changed(input.id)
                }
                KeyCode::Char(c) if !ctrl => {
                    self.values.entry(input.id).or_default().push(c);
                    Outcome::Changed(input.id)
                }
                _ => Outcome::Nothing,
            },
            Item::Check { id, .. } => match key.code {
                KeyCode::Char(' ') | KeyCode::Enter => {
                    let now = !self.checked(id);
                    self.checks.insert(id, now);
                    Outcome::Changed(id)
                }
                KeyCode::Char(c) => self.button_for(c).map_or(Outcome::Nothing, Outcome::Act),
                _ => Outcome::Nothing,
            },
            Item::Select { id, options, .. } => {
                let n = options.len().max(1);
                let at = self.choices.get(id).copied().unwrap_or(0);
                match key.code {
                    KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter => {
                        self.choices.insert(id, (at + 1) % n);
                        Outcome::Changed(id)
                    }
                    KeyCode::Left => {
                        self.choices.insert(id, (at + n - 1) % n);
                        Outcome::Changed(id)
                    }
                    KeyCode::Char(c) => self.button_for(c).map_or(Outcome::Nothing, Outcome::Act),
                    _ => Outcome::Nothing,
                }
            }
            Item::Buttons(buttons) => match key.code {
                KeyCode::Right => {
                    self.button = (self.button + 1).min(buttons.len() - 1);
                    Outcome::Nothing
                }
                KeyCode::Left => {
                    self.button = self.button.saturating_sub(1);
                    Outcome::Nothing
                }
                KeyCode::Enter => Outcome::Act(buttons[self.button.min(buttons.len() - 1)].act),
                KeyCode::Char(c) => self.button_for(c).map_or(Outcome::Nothing, Outcome::Act),
                _ => Outcome::Nothing,
            },
            Item::Row { buttons, .. } => match key.code {
                KeyCode::Enter | KeyCode::Right => Outcome::Act(buttons[0].act),
                KeyCode::Char(c) => self.button_for(c).map_or(Outcome::Nothing, Outcome::Act),
                _ => Outcome::Nothing,
            },
            _ => Outcome::Nothing,
        }
    }

    /// The finished job's id and result; `None` while it runs.
    pub fn finished(&mut self) -> Option<(&'static str, apeiron::Result<Value>)> {
        let running = self.job.as_mut()?;
        if !running.job.is_finished() {
            return None;
        }
        let mut running = self.job.take()?;
        let result = running.job.wait(None)?;
        Some((running.id, result))
    }

    /// Stops a running job and waits for it.
    pub fn stop(&mut self) {
        if let Some(mut running) = self.job.take() {
            running.job.cancel();
            running.job.wait(None);
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, w: &Words) {
        let (body, progress) = if self.job.is_some() {
            let [body, progress] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(4)]).areas(area);
            (body, Some(progress))
        } else {
            (area, None)
        };
        let (lines, focused) = self.lines(body.width as usize, w);
        let height = body.height as usize;
        if self.follow {
            if let Some((start, end)) = focused {
                if start < self.scroll {
                    self.scroll = start;
                } else if end > self.scroll + height {
                    self.scroll = end.saturating_sub(height).min(start);
                }
            }
            self.follow = false;
        }
        self.scroll = self.scroll.min(lines.len().saturating_sub(height));
        let shown: Vec<Line> = lines.into_iter().skip(self.scroll).take(height).collect();
        f.render_widget(Paragraph::new(shown), body);
        if let (Some(rect), Some(running)) = (progress, &self.job) {
            draw_progress(f, rect, &running.job.progress(), w);
        }
        if let Some((question, _)) = &self.confirm {
            let width = (area.width.saturating_sub(8)).clamp(20, 70);
            let wrapped = wrap(question, width as usize - 4);
            let rect = centered(area, width, wrapped.len() as u16 + 4);
            f.render_widget(Clear, rect);
            let block = Block::default().borders(Borders::ALL);
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let mut lines: Vec<Line> = wrapped.into_iter().map(Line::from).collect();
            lines.push(Line::default());
            lines.push(Line::styled(w.get("tui_confirm"), Tone::Muted.style()));
            f.render_widget(Paragraph::new(lines), inner);
        }
        if let Some(picker) = &self.picker {
            picker.draw(f, area, w);
        }
    }

    /// Every line of the items at `width`, and where the focused item is.
    fn lines(&self, width: usize, w: &Words) -> (Vec<Line<'static>>, Option<(usize, usize)>) {
        let width = width.max(10);
        let reversed = Style::default().add_modifier(Modifier::REVERSED);
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut focused = None;
        let mut i = 0;
        while i < self.items.len() {
            let start = out.len();
            let is_focus = i == self.focus && self.items[i].focusable();
            match &self.items[i] {
                Item::Heading(s) => {
                    if !out.is_empty() {
                        out.push(Line::default());
                    }
                    out.push(Line::styled(
                        s.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                }
                Item::Text(s, tone) => {
                    for l in wrap(s, width) {
                        out.push(Line::styled(l, tone.style()));
                    }
                }
                Item::Pre(lines) => out.extend(lines.iter().cloned()),
                Item::Gap => out.push(Line::default()),
                Item::Input(input) => {
                    let mut label = vec![Span::raw(input.label.clone())];
                    if let Some(h) = &input.hint {
                        label.push(Span::styled(format!(" — {h}"), Tone::Muted.style()));
                    }
                    if input.folder {
                        label.push(Span::styled(
                            format!(" ({})", w.get("tui_folder_key")),
                            Tone::Muted.style(),
                        ));
                    }
                    out.push(Line::from(label));
                    let value = self.value(input.id);
                    let shown = if input.secret {
                        "•".repeat(value.chars().count())
                    } else {
                        value
                    };
                    let shown = tail(&shown, width.saturating_sub(4));
                    let style = if is_focus {
                        reversed
                    } else {
                        Style::default().add_modifier(Modifier::UNDERLINED)
                    };
                    let mut field = vec![Span::raw("› "), Span::styled(shown, style)];
                    if is_focus {
                        field.push(Span::styled(" ", reversed));
                    }
                    out.push(Line::from(field));
                }
                Item::Check { id, label } => {
                    let mark = if self.checked(id) { "[x]" } else { "[ ]" };
                    let style = if is_focus { reversed } else { Style::default() };
                    out.push(Line::styled(format!("{mark} {label}"), style));
                }
                Item::Select { id, label, options } => {
                    let n = self.choices.get(id).copied().unwrap_or(0);
                    let shown = options.get(n).map(|o| o.0.clone()).unwrap_or_default();
                    let style = if is_focus { reversed } else { Style::default() };
                    out.push(Line::from(vec![
                        Span::raw(format!("{label}: ")),
                        Span::styled(format!("‹ {shown} ›"), style),
                    ]));
                }
                Item::Buttons(buttons) => {
                    let mut spans = Vec::new();
                    for (n, b) in buttons.iter().enumerate() {
                        let style = if is_focus && n == self.button {
                            reversed
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        };
                        spans.push(Span::styled(format!("[{}] {}", b.key, b.label), style));
                        spans.push(Span::raw("  "));
                    }
                    out.push(Line::from(spans));
                }
                Item::Row { .. } => {
                    // A table: the rows that follow each other.
                    let end = (i..self.items.len())
                        .find(|&j| !matches!(self.items[j], Item::Row { .. }))
                        .unwrap_or(self.items.len());
                    let widths = column_widths(&self.items[i..end], width);
                    for j in i..end {
                        let Item::Row {
                            cells,
                            tone,
                            buttons,
                        } = &self.items[j]
                        else {
                            continue;
                        };
                        if j == self.focus && !buttons.is_empty() {
                            focused = Some((out.len(), out.len() + 1));
                        }
                        out.push(row_line(
                            cells,
                            buttons,
                            &widths,
                            *tone,
                            j == self.focus,
                            width,
                        ));
                    }
                    i = end;
                    continue;
                }
            }
            if is_focus {
                focused = Some((start, out.len()));
            }
            i += 1;
        }
        (out, focused)
    }
}

/// What a focused item is, to find it again among new items.
fn identity(item: &Item) -> Option<(&'static str, usize)> {
    match item {
        Item::Input(input) => Some((input.id, usize::MAX)),
        Item::Check { id, .. } | Item::Select { id, .. } => Some((id, usize::MAX)),
        Item::Row { buttons, .. } | Item::Buttons(buttons) => {
            buttons.first().map(|b| (b.act.id, b.act.arg))
        }
        _ => None,
    }
}

fn draw_progress(f: &mut Frame, area: Rect, p: &apeiron::Progress, w: &Words) {
    let phase = serde_json::to_value(p.phase)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let ratio = match (p.bytes_total, p.items_total) {
        (Some(total), _) if total > 0 => p.bytes_done as f64 / total as f64,
        (_, Some(total)) if total > 0 => p.items_done as f64 / total as f64,
        _ => 0.0,
    };
    let items = match p.items_total {
        Some(total) => w.with(
            "progress_items",
            &[
                ("done", p.items_done.to_string()),
                ("total", total.to_string()),
            ],
        ),
        None => w.with("progress_items_open", &[("done", p.items_done.to_string())]),
    };
    let current = p
        .current
        .as_ref()
        .map(|c| format!(" · {}", String::from_utf8_lossy(c.as_bytes())))
        .unwrap_or_default();
    let counts = format!("{items} · {}{current}", format::bytes(p.bytes_done, w.lang));
    let [title, gauge, line, keys] = Layout::vertical([Constraint::Length(1); 4]).areas(area);
    f.render_widget(
        Paragraph::new(Line::styled(
            w.get(&format!("phase_{phase}")),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        title,
    );
    f.render_widget(
        Gauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .gauge_style(Style::default().fg(Color::Cyan)),
        gauge,
    );
    f.render_widget(
        Paragraph::new(Line::styled(
            tail(&counts, line.width as usize),
            Tone::Muted.style(),
        )),
        line,
    );
    f.render_widget(
        Paragraph::new(Line::styled(w.get("tui_stop"), Tone::Muted.style())),
        keys,
    );
}

/// The widths of the columns of `rows`, narrowed from the widest so the
/// rows with their buttons fit `width` when they can.
fn column_widths(rows: &[Item], width: usize) -> Vec<usize> {
    let mut widths: Vec<usize> = Vec::new();
    let mut buttons_width = 0;
    for r in rows {
        if let Item::Row { cells, buttons, .. } = r {
            for (n, c) in cells.iter().enumerate() {
                let cw = c.width();
                match widths.get_mut(n) {
                    Some(w) => *w = (*w).max(cw),
                    None => widths.push(cw),
                }
            }
            let b: usize = buttons.iter().map(|b| b.label.width() + 6).sum();
            buttons_width = buttons_width.max(b);
        }
    }
    let total = |w: &[usize]| w.iter().map(|c| c + 2).sum::<usize>() + buttons_width;
    while total(&widths) > width {
        let Some((n, widest)) = widths.iter().copied().enumerate().max_by_key(|(_, w)| *w) else {
            break;
        };
        if widest <= 8 {
            break;
        }
        widths[n] = widest - 1;
    }
    widths
}

fn row_line(
    cells: &[String],
    buttons: &[Button],
    widths: &[usize],
    tone: Tone,
    focused: bool,
    width: usize,
) -> Line<'static> {
    let mut text = String::new();
    for (n, c) in cells.iter().enumerate() {
        let w = widths.get(n).copied().unwrap_or(0);
        let cut = cut(c, w);
        let pad = w.saturating_sub(cut.width());
        text.push_str(&cut);
        text.push_str(&" ".repeat(pad + 2));
    }
    for b in buttons {
        text.push_str(&format!("[{}] {}  ", b.key, b.label));
    }
    let text = cut(text.trim_end(), width);
    let style = if focused && !buttons.is_empty() {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        tone.style()
    };
    Line::styled(text, style)
}

/// `s` cut to `width` columns, with `…` when it was cut.
fn cut(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw + 1 > width {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// The end of `s` that fits `width` columns, so the cursor stays in sight.
fn tail(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_owned();
    }
    let mut out: Vec<char> = Vec::new();
    let mut used = 1;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if used + cw > width {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out.into_iter().rev().collect()
}

/// `s` broken into lines of at most `width` columns, at spaces where it can.
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for paragraph in s.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let needed = if line.is_empty() {
                word.width()
            } else {
                line.width() + 1 + word.width()
            };
            if needed > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
            while line.width() > width {
                let mut head = String::new();
                let mut rest = String::new();
                let mut used = 0;
                for c in line.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw <= width && rest.is_empty() {
                        head.push(c);
                        used += cw;
                    } else {
                        rest.push(c);
                    }
                }
                out.push(head);
                line = rest;
            }
        }
        out.push(line);
    }
    out
}

/// The last part of a path, as the pages name a folder.
pub fn base_name(path: &str) -> String {
    path.split(['/', '\\'])
        .rfind(|p| !p.is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_words_and_lines_wrap() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(tail("abcdef", 4), "…def");
    }

    #[test]
    fn keys_move_between_fields_and_press_buttons() {
        let mut page = Page::default();
        page.show(vec![
            Item::Heading("h".into()),
            Item::Input(input("a", "A").submit(act("go"))),
            Item::Check {
                id: "c",
                label: "C".into(),
            },
            Item::Buttons(vec![
                button('g', "Go", act("go")),
                button('x', "X", act("x")),
            ]),
        ]);
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(page.key(key(KeyCode::Char('h'))), Outcome::Changed("a"));
        assert_eq!(page.key(key(KeyCode::Char('i'))), Outcome::Changed("a"));
        assert_eq!(page.value("a"), "hi");
        assert_eq!(page.key(key(KeyCode::Enter)), Outcome::Act(act("go")));
        page.key(key(KeyCode::Tab));
        page.key(key(KeyCode::Char(' ')));
        assert!(page.checked("c"));
        // A letter presses its button when no field takes it.
        assert_eq!(page.key(key(KeyCode::Char('x'))), Outcome::Act(act("x")));
        page.key(key(KeyCode::Tab));
        page.key(key(KeyCode::Right));
        assert_eq!(page.key(key(KeyCode::Enter)), Outcome::Act(act("x")));
        assert_eq!(page.key(key(KeyCode::Esc)), Outcome::Back);
        // A question waits for y.
        page.ask("sure?".into(), act("go"));
        assert_eq!(page.key(key(KeyCode::Char('n'))), Outcome::Nothing);
        page.ask("sure?".into(), act("go"));
        assert_eq!(page.key(key(KeyCode::Char('y'))), Outcome::Act(act("go")));
    }
}
