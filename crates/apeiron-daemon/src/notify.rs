//! A desktop notification when a job fails (DAEMON.md §4). It goes through
//! the system's own tool, so a computer without one, such as a server, simply
//! shows nothing; the log and `daemon status` still say what happened.

use std::process::{Command, Stdio};

use crate::state::{JobState, RunResult};

/// Whether a run should be told: a failure that follows a run that did not
/// fail, so a job retried every hour does not repeat it.
pub(crate) fn worth_telling(previous: Option<&JobState>, now: &JobState) -> bool {
    now.result == RunResult::Failed && previous.is_none_or(|p| p.result != RunResult::Failed)
}

/// Shows `title` and `body`; waits for nothing and reports nothing.
pub(crate) fn show(title: &str, body: &str) {
    let Some(mut command) = command(title, body) else {
        return;
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = command.spawn() {
        // Reap it in the background, so no finished process is left over.
        std::thread::spawn(move || child.wait());
    }
}

/// The text is passed as arguments or environment variables, never inside
/// a script, so a job name or message cannot change what runs.
fn command(title: &str, body: &str) -> Option<Command> {
    if cfg!(target_os = "macos") {
        let mut c = Command::new("osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            title,
            body,
        ]);
        Some(c)
    } else if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; \
             $n = New-Object System.Windows.Forms.NotifyIcon; \
             $n.Icon = [System.Drawing.SystemIcons]::Warning; $n.Visible = $true; \
             $n.ShowBalloonTip(10000, $env:APEIRON_TITLE, $env:APEIRON_BODY, 'Error'); \
             Start-Sleep -Seconds 10; $n.Dispose()",
        ])
        .env("APEIRON_TITLE", title)
        .env("APEIRON_BODY", body);
        Some(c)
    } else if cfg!(unix) {
        let mut c = Command::new("notify-send");
        c.args([
            "--app-name=Apeiron",
            "--urgency=critical",
            "--",
            title,
            body,
        ]);
        Some(c)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(result: RunResult) -> JobState {
        JobState {
            last_start: jiff::Timestamp::UNIX_EPOCH,
            last_end: jiff::Timestamp::UNIX_EPOCH,
            result,
            message: String::new(),
        }
    }

    #[test]
    fn only_the_first_failure_in_a_row_is_told() {
        let (ok, failed) = (run(RunResult::Ok), run(RunResult::Failed));
        assert!(worth_telling(None, &failed));
        assert!(worth_telling(Some(&ok), &failed));
        assert!(!worth_telling(Some(&failed), &failed));
        assert!(!worth_telling(Some(&failed), &ok));
        assert!(!worth_telling(None, &run(RunResult::Locked)));
    }
}
