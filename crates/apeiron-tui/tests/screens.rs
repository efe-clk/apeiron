//! The terminal interface driven by keys, as a user would (TUI.md): every
//! screen on a real repository, drawn into a test terminal.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use apeiron::{BackupRequest, Repository};
use apeiron_tui::{App, Options};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

struct Ui {
    app: App,
    terminal: Terminal<TestBackend>,
}

impl Ui {
    fn new(options: Options) -> Self {
        Ui {
            app: App::new(options),
            terminal: Terminal::new(TestBackend::new(120, 50)).unwrap(),
        }
    }

    /// Everything on the screen, line by line.
    fn screen(&mut self) -> String {
        self.terminal.draw(|f| self.app.draw(f)).unwrap();
        let buffer = self.terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn shows(&mut self, text: &str) {
        let screen = self.screen();
        assert!(
            screen.contains(text),
            "{text:?} is not on the screen:\n{screen}"
        );
    }

    fn press(&mut self, code: KeyCode) {
        self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
        self.app.tick();
        self.screen();
    }

    fn ctrl(&mut self, c: char) {
        self.app
            .key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        self.screen();
    }

    fn typing(&mut self, text: &str) {
        for c in text.chars() {
            self.press(KeyCode::Char(c));
        }
    }

    /// Replaces the focused field's text.
    fn retype(&mut self, text: &str) {
        self.ctrl('u');
        self.typing(text);
    }

    /// Ticks until `text` shows, as a running job ends.
    fn wait_for(&mut self, text: &str) {
        let start = Instant::now();
        while !self.screen().contains(text) {
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "{text:?} never showed:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(20));
            self.app.tick();
        }
    }

    /// Opens a screen of the menu by its name.
    fn menu(&mut self, name: &str) {
        // Esc until the menu has the keys: back from each pushed screen.
        for _ in 0..6 {
            if self.screen().contains("choose a screen") {
                break;
            }
            self.press(KeyCode::Esc);
        }
        for _ in 0..12 {
            self.press(KeyCode::Up);
        }
        for _ in 0..12 {
            let screen = self.screen();
            let line = screen
                .lines()
                .find(|l| l.contains(name))
                .unwrap_or_else(|| panic!("{name} is not in the menu:\n{screen}"));
            // The chosen menu item is drawn reversed; the test terminal
            // keeps its style, so look at the buffer.
            let y = screen.lines().position(|l| l == line).unwrap() as u16;
            let buffer = self.terminal.backend().buffer();
            if buffer[(2, y)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
            {
                self.press(KeyCode::Enter);
                return;
            }
            self.press(KeyCode::Down);
        }
        panic!("could not open {name}");
    }
}

fn options(root: &Path, repo: Option<Repository>) -> Options {
    Options {
        repo,
        memory: Some(root.join("desktop.json")),
        schedule: Some(apeiron_daemon::Paths {
            jobs: root.join("jobs.toml"),
            state_dir: root.join("state"),
            scope: None,
        }),
        plugins: Some(root.join("plugins")),
        lang: "en".to_owned(),
    }
}

fn path(p: PathBuf) -> String {
    p.display().to_string()
}

#[test]
fn a_user_backs_up_looks_and_restores_with_keys_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("Belgeler/notes")).unwrap();
    fs::write(root.join("Belgeler/a.txt"), "one\n").unwrap();
    fs::write(root.join("Belgeler/notes/b.txt"), "two\n").unwrap();
    let mut ui = Ui::new(options(root, None));

    // Start: a new encrypted repository; its recovery key is shown once.
    ui.shows("Make a new backup folder");
    ui.press(KeyCode::Tab);
    ui.press(KeyCode::Tab);
    ui.typing(&path(root.join("repo")));
    ui.press(KeyCode::Tab);
    ui.typing("correct horse battery");
    ui.press(KeyCode::Tab);
    ui.typing("correct horse batterx");
    ui.press(KeyCode::Enter);
    ui.shows("The two passwords are not the same.");
    ui.press(KeyCode::Backspace);
    ui.typing("y");
    ui.press(KeyCode::Enter);
    ui.shows("Write down your recovery key");
    ui.press(KeyCode::Char('d'));
    ui.shows("There are no backups yet.");

    // Back up: the name follows the folder.
    ui.press(KeyCode::Char('b'));
    ui.shows("Back up a folder");
    ui.typing(&path(root.join("Belgeler")));
    ui.shows("backup/belgeler");
    ui.press(KeyCode::Enter);
    ui.wait_for("Backed up. Files: 2");
    fs::write(root.join("Belgeler/a.txt"), "one\nmore\n").unwrap();
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Enter);
    ui.wait_for("newly stored");

    // The versions, what changed and what is inside.
    ui.press(KeyCode::Esc);
    ui.press(KeyCode::Char('v'));
    ui.shows("Versions of belgeler");
    ui.press(KeyCode::Char('c'));
    ui.shows("[changed] a.txt");
    ui.press(KeyCode::Esc);
    ui.press(KeyCode::Char('b'));
    ui.shows("a.txt");
    ui.shows("notes");
    ui.press(KeyCode::Down);
    ui.press(KeyCode::Char('o'));
    ui.shows("b.txt");
    ui.press(KeyCode::Char('r'));
    ui.shows("notes/b.txt from the version of");
    ui.retype(&path(root.join("out")));
    ui.press(KeyCode::Enter);
    ui.wait_for("Restored. Files: 1");
    let restored = fs::read_dir(root.join("out"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(fs::read(restored.path().join("b.txt")).unwrap(), b"two\n");

    // Check.
    ui.menu("Check");
    ui.shows("Check the backups");
    ui.press(KeyCode::Char('g'));
    ui.wait_for("Everything is healthy");

    // The folder browser chooses a folder for a field.
    ui.menu("Back up a folder");
    ui.press(KeyCode::F(2));
    ui.shows("Choose this folder");
    ui.press(KeyCode::Enter);
    assert!(ui.screen().contains(&path(dirs::home_dir().unwrap())));

    // Closing leads to the start, which remembers the repository.
    ui.menu("Close");
    ui.shows(&format!("Last used: {}", path(root.join("repo"))));
    ui.press(KeyCode::Enter);
    ui.shows("This backup folder is locked.");
    ui.typing("correct horse battery");
    ui.press(KeyCode::Enter);
    ui.shows("belgeler");
    ui.ctrl('c');
    assert!(ui.app.done());
}

#[test]
fn the_other_screens_do_what_their_pages_do() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (repo, _) = Repository::init_encrypted(root.join("repo"), "correct horse battery").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.txt"), "one\n").unwrap();
    for name in ["main", "topic"] {
        let mut request = BackupRequest::new(root.join("src"), name);
        request.meta = vec![("source".to_owned(), path(root.join("src")))];
        repo.backup(request).unwrap().wait().unwrap();
        fs::write(root.join("src/a.txt"), "one\ntwo\n").unwrap();
    }
    let mut ui = Ui::new(options(root, Some(repo)));
    ui.shows("topic");

    // Passwords: a new recovery key is shown.
    ui.menu("Passwords");
    ui.shows("Password (used");
    for _ in 0..4 {
        ui.press(KeyCode::Tab);
    }
    ui.press(KeyCode::Char('k'));
    ui.shows("Write down your recovery key");
    ui.shows("Recovery key");
    // Removing a key waits for y.
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Char('x'));
    ui.shows("y: yes");
    ui.press(KeyCode::Char('n'));

    // Retention: added after the warning.
    ui.menu("Retention");
    ui.shows("There are no rules yet.");
    ui.typing("topic");
    ui.press(KeyCode::Tab);
    ui.typing("3");
    ui.press(KeyCode::Enter);
    ui.shows("This rule can never be removed");
    ui.press(KeyCode::Char('y'));
    ui.shows("3 days");

    // Report.
    ui.menu("Report");
    ui.press(KeyCode::Char('g'));
    ui.wait_for("Findings");
    ui.shows("There is no remote");

    // Sync: a remote is added and removed.
    ui.menu("Sync");
    ui.shows("Add a remote below first");
    ui.typing("nas");
    ui.press(KeyCode::Tab);
    ui.typing(&path(root.join("nas")));
    ui.press(KeyCode::Enter);
    ui.shows("Sync now");
    // The focus stays on the address field; the remote is the row above
    // the name field.
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Up);
    ui.press(KeyCode::Char('x'));
    ui.shows("Remove the remote nas?");
    ui.press(KeyCode::Char('y'));
    ui.shows("Add a remote below first");

    // Plugins.
    ui.menu("Plugins");
    ui.shows("No plugin is installed.");

    // Schedule: the jobs file is edited and saved only when valid.
    ui.menu("Schedule");
    ui.shows("No job is scheduled yet.");
    ui.press(KeyCode::Char('e'));
    let example = ui.app.take_edit().unwrap();
    assert!(example.contains(&path(root.join("repo"))), "{example}");
    ui.app.edited(Ok("[job.x]\ndo = \"nothing\"\n".to_owned()));
    ui.shows("Nothing was saved.");
    ui.press(KeyCode::Char('e'));
    assert!(ui.app.take_edit().unwrap().contains("nothing"));
    let jobs = format!(
        "[job.docs]\nrepo = {:?}\ndo = \"verify\"\nevery = \"day\"\nat = \"02:30\"\n",
        path(root.join("repo"))
    );
    ui.app.edited(Ok(jobs));
    ui.shows("Saved. 1 jobs are scheduled.");
    ui.shows("docs");
    ui.shows("not yet");

    // Reviews: one is asked for, commented and closed.
    ui.menu("Reviews");
    ui.shows("No open reviews.");
    ui.press(KeyCode::Tab);
    ui.press(KeyCode::Tab);
    ui.press(KeyCode::Tab);
    ui.typing("Two lines");
    ui.press(KeyCode::Enter);
    ui.shows("topic → main");
    ui.typing("looks right");
    ui.press(KeyCode::Tab);
    ui.press(KeyCode::Tab);
    ui.press(KeyCode::Char('c'));
    ui.shows("commented");
    ui.shows("looks right");
    ui.press(KeyCode::Char('x'));
    ui.shows("Closed");
    ui.press(KeyCode::Esc);
    ui.shows("No open reviews.");
}

#[test]
fn the_screens_speak_turkish() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = options(dir.path(), None);
    options.lang = "tr".to_owned();
    let mut ui = Ui::new(options);
    ui.shows("Yedekleriniz");
    ui.shows("Ctrl+C çık");
}
