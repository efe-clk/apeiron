//! Webhooks (DAEMON.md §8): a message to a chat or an alerting service when
//! a job starts failing, recovers or ends. They come from a file of their
//! own beside the jobs file, which only whoever runs the service writes, so
//! the Schedule page cannot make the server call an address.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::{JobState, RunResult};
use crate::{Error, IoContext, Result};

const PAGERDUTY: &str = "https://events.pagerduty.com/v2/enqueue";
const TIMEOUT: Duration = Duration::from_secs(10);

/// One webhook of the webhooks file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Webhook {
    pub name: String,
    pub url: String,
    pub format: Format,
    pub on: Vec<Event>,
    /// Only these jobs; every job when empty.
    pub jobs: Vec<String>,
    /// For PagerDuty: the integration's routing key.
    pub routing_key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    Slack,
    Teams,
    Pagerduty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Event {
    /// A run failed after one that did not.
    Failed,
    /// A run succeeded after one that failed.
    Recovered,
    /// Every run, whatever its result.
    Done,
    /// `apeiron daemon test-webhooks`.
    Test,
}

impl Event {
    fn name(self) -> &'static str {
        match self {
            Event::Failed => "failed",
            Event::Recovered => "recovered",
            Event::Done => "done",
            Event::Test => "test",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    webhook: BTreeMap<String, RawWebhook>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWebhook {
    url: Option<String>,
    url_file: Option<String>,
    #[serde(default = "json_format")]
    format: Format,
    on: Option<Vec<Event>>,
    #[serde(default)]
    jobs: Vec<String>,
    routing_key_file: Option<String>,
}

fn json_format() -> Format {
    Format::Json
}

/// Reads the webhooks file; a missing file has none.
pub fn load_webhooks(path: &Path) -> Result<Vec<Webhook>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).at(path),
    };
    let invalid = |message: String| Error::Jobs {
        path: path.to_path_buf(),
        message,
    };
    let raw: RawFile = toml::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    raw.webhook
        .into_iter()
        .map(|(name, raw)| {
            to_webhook(&name, raw).map_err(|e| invalid(format!("webhook {name:?}: {e}")))
        })
        .collect()
}

/// The first line of a file, as for `password_file`; `~` is the home folder.
fn first_line(file: &str) -> std::result::Result<String, String> {
    let path = crate::jobs::expand(file);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let line = text.lines().next().unwrap_or_default().trim().to_owned();
    if line.is_empty() {
        return Err(format!("{} is empty", path.display()));
    }
    Ok(line)
}

fn to_webhook(name: &str, raw: RawWebhook) -> std::result::Result<Webhook, String> {
    let url = match (raw.url, raw.url_file) {
        (Some(_), Some(_)) => return Err("give `url` or `url_file`, not both".to_owned()),
        (Some(url), None) => url,
        (None, Some(file)) => first_line(&file)?,
        (None, None) if raw.format == Format::Pagerduty => PAGERDUTY.to_owned(),
        (None, None) => return Err("`url` or `url_file` is missing".to_owned()),
    };
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("the address must start with https:// or http://".to_owned());
    }
    let routing_key = raw
        .routing_key_file
        .as_deref()
        .map(first_line)
        .transpose()?;
    if (raw.format == Format::Pagerduty) != routing_key.is_some() {
        return Err(
            "`routing_key_file` is for `format = \"pagerduty\"`, which needs it".to_owned(),
        );
    }
    let on = raw
        .on
        .unwrap_or_else(|| vec![Event::Failed, Event::Recovered]);
    if on.is_empty() || on.contains(&Event::Test) {
        return Err("`on` lists failed, recovered or done".to_owned());
    }
    if raw.format == Format::Pagerduty && on.contains(&Event::Done) {
        return Err("PagerDuty is told only when a job fails and recovers".to_owned());
    }
    Ok(Webhook {
        name: name.to_owned(),
        url,
        format: raw.format,
        on,
        jobs: raw.jobs,
        routing_key,
    })
}

/// The events a run makes, after `previous`.
pub(crate) fn events(previous: Option<&JobState>, now: &JobState) -> Vec<Event> {
    let was_failing = previous.is_some_and(|p| p.result == RunResult::Failed);
    let mut events = vec![Event::Done];
    match now.result {
        RunResult::Failed if !was_failing => events.push(Event::Failed),
        RunResult::Ok | RunResult::Warnings if was_failing => events.push(Event::Recovered),
        _ => {}
    }
    events
}

/// What a webhook is told about.
pub(crate) struct Message<'a> {
    pub event: Event,
    pub job: &'a str,
    pub action: &'a str,
    pub run: Option<&'a JobState>,
    pub host: &'a str,
}

impl Webhook {
    /// Whether the webhook wants `event` of `job`.
    pub(crate) fn wants(&self, event: Event, job: &str) -> bool {
        (event == Event::Test || self.on.contains(&event))
            && (self.jobs.is_empty() || self.jobs.iter().any(|j| j == job))
    }

    /// Sends `message`; the error says why it was not accepted.
    pub(crate) fn send(&self, message: &Message) -> std::result::Result<(), String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        for body in self.bodies(message) {
            let response = client
                .post(&self.url)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .map_err(|e| e.to_string())?;
            if !response.status().is_success() {
                return Err(format!("the service answered {}", response.status()));
            }
        }
        Ok(())
    }

    /// The JSON bodies of a message; PagerDuty gets two for a test, so the
    /// alert it opens is closed again.
    fn bodies(&self, m: &Message) -> Vec<Value> {
        let result = m.run.map(|r| match r.result {
            RunResult::Ok => "ok",
            RunResult::Warnings => "warnings",
            RunResult::Failed => "failed",
            RunResult::Locked => "locked",
        });
        let message = m.run.map(|r| r.message.as_str()).unwrap_or_default();
        let what = match m.event {
            Event::Failed => "failed".to_owned(),
            Event::Recovered => "succeeds again".to_owned(),
            Event::Done => format!("ended: {}", result.unwrap_or_default()),
            Event::Test => "is a test of this webhook".to_owned(),
        };
        let text = if message.is_empty() {
            format!("Apeiron on {}: job {} {what}", m.host, m.job)
        } else {
            format!("Apeiron on {}: job {} {what}: {message}", m.host, m.job)
        };
        match self.format {
            Format::Json => vec![json!({
                "event": m.event.name(),
                "job": m.job,
                "do": m.action,
                "result": result,
                "message": message,
                "start": m.run.map(|r| r.last_start),
                "end": m.run.map(|r| r.last_end),
                "host": m.host,
            })],
            Format::Slack => vec![json!({ "text": text })],
            Format::Teams => vec![json!({
                "type": "message",
                "attachments": [{
                    "contentType": "application/vnd.microsoft.card.adaptive",
                    "content": {
                        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                        "type": "AdaptiveCard",
                        "version": "1.4",
                        "body": [{ "type": "TextBlock", "text": text, "wrap": true }],
                    },
                }],
            })],
            Format::Pagerduty => {
                let event = |action: &str, severity: &str| {
                    json!({
                        "routing_key": self.routing_key,
                        "event_action": action,
                        "dedup_key": format!("apeiron/{}/{}", m.host, m.job),
                        "payload": {
                            "summary": text,
                            "source": m.host,
                            "severity": severity,
                            "component": m.job,
                        },
                    })
                };
                match m.event {
                    Event::Recovered => vec![event("resolve", "info")],
                    Event::Test => vec![event("trigger", "info"), event("resolve", "info")],
                    _ => vec![event("trigger", "error")],
                }
            }
        }
    }
}

/// The name of this computer, as webhooks show it.
pub(crate) fn host() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> std::result::Result<Vec<Webhook>, String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.webhooks.toml");
        fs::write(&path, text).unwrap();
        load_webhooks(&path).map_err(|e| e.to_string())
    }

    fn run(result: RunResult) -> JobState {
        JobState {
            last_start: jiff::Timestamp::UNIX_EPOCH,
            last_end: jiff::Timestamp::UNIX_EPOCH,
            result,
            message: "a message".to_owned(),
            last_success: None,
        }
    }

    #[test]
    fn the_file_is_read_and_checked() {
        let hooks = parse(
            "[webhook.chat]\nurl = \"https://hooks.example/1\"\nformat = \"slack\"\n\
             [webhook.all]\nurl = \"http://localhost:9/x\"\non = [\"done\"]\njobs = [\"docs\"]\n",
        )
        .unwrap();
        assert_eq!(hooks.len(), 2);
        let chat = hooks.iter().find(|h| h.name == "chat").unwrap();
        assert_eq!(chat.on, [Event::Failed, Event::Recovered]);
        assert!(chat.wants(Event::Failed, "any"));
        assert!(!chat.wants(Event::Done, "any"));
        let all = hooks.iter().find(|h| h.name == "all").unwrap();
        assert!(all.wants(Event::Done, "docs") && !all.wants(Event::Done, "other"));
        assert!(parse("").unwrap().is_empty());

        for (text, why) in [
            ("[webhook.a]\nformat = \"slack\"\n", "missing"),
            ("[webhook.a]\nurl = \"ftp://x\"\n", "https://"),
            ("[webhook.a]\nurl = \"https://x\"\non = []\n", "`on`"),
            ("[webhook.a]\nformat = \"pagerduty\"\n", "routing_key_file"),
            (
                "[webhook.a]\nurl = \"https://x\"\nother = 1\n",
                "unknown field",
            ),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains(why), "{text}: {e}");
        }
    }

    #[test]
    fn failing_and_recovering_are_told_once() {
        let (ok, failed) = (run(RunResult::Ok), run(RunResult::Failed));
        assert_eq!(events(None, &failed), [Event::Done, Event::Failed]);
        assert_eq!(events(Some(&failed), &failed), [Event::Done]);
        assert_eq!(events(Some(&failed), &ok), [Event::Done, Event::Recovered]);
        assert_eq!(events(Some(&ok), &ok), [Event::Done]);
    }

    #[test]
    fn each_format_has_its_own_body() {
        let r = run(RunResult::Failed);
        let m = Message {
            event: Event::Failed,
            job: "docs",
            action: "backup",
            run: Some(&r),
            host: "nas",
        };
        let hook = |format, routing_key: Option<&str>| Webhook {
            name: "h".to_owned(),
            url: "https://x".to_owned(),
            format,
            on: vec![Event::Failed],
            jobs: Vec::new(),
            routing_key: routing_key.map(str::to_owned),
        };
        let json = hook(Format::Json, None).bodies(&m);
        assert_eq!(json[0]["event"], "failed");
        assert_eq!(json[0]["result"], "failed");
        assert_eq!(json[0]["host"], "nas");
        let slack = hook(Format::Slack, None).bodies(&m);
        assert_eq!(
            slack[0]["text"],
            "Apeiron on nas: job docs failed: a message"
        );
        let teams = hook(Format::Teams, None).bodies(&m);
        assert_eq!(
            teams[0]["attachments"][0]["content"]["body"][0]["text"],
            slack[0]["text"]
        );
        let pd = hook(Format::Pagerduty, Some("key"));
        assert_eq!(pd.bodies(&m)[0]["event_action"], "trigger");
        assert_eq!(pd.bodies(&m)[0]["routing_key"], "key");
        assert_eq!(pd.bodies(&m)[0]["dedup_key"], "apeiron/nas/docs");
        let test = Message {
            event: Event::Test,
            ..m
        };
        let actions: Vec<_> = pd
            .bodies(&test)
            .iter()
            .map(|b| b["event_action"].clone())
            .collect();
        assert_eq!(actions, ["trigger", "resolve"]);
    }
}
