//! Metrics in the Prometheus text format (DAEMON.md §7): the last run of
//! every job, worked out from the state file whenever they are asked for.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use jiff::Timestamp;

use crate::daemon::{Paths, status};
use crate::state::RunResult;
use crate::{Error, Result};

/// The `Content-Type` of the text format.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Metrics being written, one family after another.
#[derive(Debug, Default)]
pub struct Metrics {
    text: String,
}

/// The labels and the value of one sample.
pub type Sample = (Vec<(&'static str, String)>, f64);

impl Metrics {
    /// Adds a gauge with its help text and samples; a gauge without samples
    /// is left out.
    pub fn gauge(&mut self, name: &str, help: &str, samples: impl IntoIterator<Item = Sample>) {
        let mut lines = String::new();
        for (labels, value) in samples {
            lines.push_str(name);
            if !labels.is_empty() {
                let labels: Vec<String> = labels
                    .iter()
                    .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
                    .collect();
                let _ = write!(lines, "{{{}}}", labels.join(","));
            }
            let _ = writeln!(lines, " {value}");
        }
        if !lines.is_empty() {
            let _ = writeln!(self.text, "# HELP {name} {help}");
            let _ = writeln!(self.text, "# TYPE {name} gauge");
            self.text.push_str(&lines);
        }
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

/// A label value: `\`, `"` and line breaks escaped.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn seconds(t: Timestamp) -> f64 {
    t.as_millisecond() as f64 / 1000.0
}

/// The metrics of the jobs of `paths`, and the version of the program.
pub fn job_metrics(paths: &Paths) -> Result<Metrics> {
    let jobs = status(paths)?;
    let mut m = Metrics::default();
    m.gauge(
        "apeiron_build_info",
        "The version of Apeiron, as a label.",
        [(vec![("version", env!("CARGO_PKG_VERSION").to_owned())], 1.0)],
    );
    m.gauge(
        "apeiron_jobs",
        "Jobs in the jobs file.",
        [(vec![], jobs.len() as f64)],
    );
    let labels = |j: &crate::JobStatus| vec![("job", j.name.clone()), ("do", j.action.clone())];
    let last = |f: &dyn Fn(&crate::JobState) -> Option<f64>| -> Vec<Sample> {
        jobs.iter()
            .filter_map(|j| Some((labels(j), f(j.last.as_ref()?)?)))
            .collect()
    };
    m.gauge(
        "apeiron_job_last_start_timestamp_seconds",
        "When the last run of the job started.",
        last(&|s| Some(seconds(s.last_start))),
    );
    m.gauge(
        "apeiron_job_last_end_timestamp_seconds",
        "When the last run of the job ended.",
        last(&|s| Some(seconds(s.last_end))),
    );
    m.gauge(
        "apeiron_job_last_duration_seconds",
        "How long the last run of the job took.",
        last(&|s| Some(seconds(s.last_end) - seconds(s.last_start))),
    );
    m.gauge(
        "apeiron_job_last_success_timestamp_seconds",
        "When the last run of the job that succeeded ended.",
        last(&|s| s.last_success.map(seconds)),
    );
    m.gauge(
        "apeiron_job_last_result",
        "The result of the last run of the job, as a label: ok, warnings, failed or locked.",
        jobs.iter().filter_map(|j| {
            let result = match j.last.as_ref()?.result {
                RunResult::Ok => "ok",
                RunResult::Warnings => "warnings",
                RunResult::Failed => "failed",
                RunResult::Locked => "locked",
            };
            let mut l = labels(j);
            l.push(("result", result.to_owned()));
            Some((l, 1.0))
        }),
    );
    m.gauge(
        "apeiron_job_failing",
        "1 when the last run of the job failed, otherwise 0.",
        jobs.iter().map(|j| {
            let failed = j
                .last
                .as_ref()
                .is_some_and(|s| s.result == RunResult::Failed);
            (labels(j), if failed { 1.0 } else { 0.0 })
        }),
    );
    m.gauge(
        "apeiron_job_next_run_timestamp_seconds",
        "When the job runs next.",
        jobs.iter().map(|j| (labels(j), seconds(j.next_run))),
    );
    Ok(m)
}

/// [`job_metrics`] as text, for `apeiron daemon metrics`.
pub fn metrics(paths: &Paths) -> Result<String> {
    Ok(job_metrics(paths)?.into_text())
}

/// Answers `GET /metrics` on `listener` with the metrics of `paths`, on a
/// thread of its own that lasts as long as the program (DAEMON.md §7).
pub fn serve_metrics(listener: TcpListener, paths: Paths) -> Result<()> {
    std::thread::Builder::new()
        .name("apeiron-metrics".to_owned())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &paths);
            }
        })
        .map(drop)
        .map_err(|e| Error::Io {
            path: "metrics".into(),
            source: e,
        })
}

/// Reads one request and answers it; a slow or broken client is dropped.
fn answer(mut stream: TcpStream, paths: &Paths) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let line = request.split(|&b| b == b'\r').next().unwrap_or_default();
    let target = line.split(|&b| b == b' ').nth(1).unwrap_or_default();
    let (status, kind, body) = match (line.starts_with(b"GET "), target) {
        (true, b"/metrics") => match metrics(paths) {
            Ok(text) => ("200 OK", CONTENT_TYPE, text),
            Err(e) => ("500 Internal Server Error", "text/plain", format!("{e}\n")),
        },
        _ => ("404 Not Found", "text/plain", "not found\n".to_owned()),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}
