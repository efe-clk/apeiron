//! OpenTelemetry traces (DAEMON.md §9): a span for each job of the
//! background service and each operation of a web server, sent as OTLP over
//! HTTP with JSON to the collector `OTEL_EXPORTER_OTLP_ENDPOINT` names. Off
//! when it is not set. Spans are sent in batches from a thread of their own,
//! so a slow collector never holds up a job or a request.

use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// One finished operation.
#[derive(Clone, Debug)]
pub struct Span {
    pub name: String,
    pub start: SystemTime,
    pub end: SystemTime,
    /// `None` when it succeeded, else why it failed.
    pub error: Option<String>,
    pub attributes: Vec<(String, Value)>,
}

enum Message {
    Span(Span),
    Flush(Sender<()>),
}

static SENDER: OnceLock<Option<Sender<Message>>> = OnceLock::new();

fn sender() -> Option<&'static Sender<Message>> {
    SENDER
        .get_or_init(|| {
            let endpoint = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
                .ok()
                .filter(|e| !e.is_empty())
                .or_else(|| {
                    std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                        .ok()
                        .filter(|e| !e.is_empty())
                        .map(|e| format!("{}/v1/traces", e.trim_end_matches('/')))
                })?;
            let (tx, rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("apeiron-otel".to_owned())
                .spawn(move || export(&endpoint, &rx))
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

/// Whether spans are sent anywhere.
pub fn enabled() -> bool {
    sender().is_some()
}

/// Sends a span, when traces are on.
pub fn record(span: Span) {
    if let Some(tx) = sender() {
        let _ = tx.send(Message::Span(span));
    }
}

/// Waits until the spans recorded so far are sent, at most five seconds;
/// for a program that is about to end.
pub fn flush() {
    if let Some(tx) = sender() {
        let (done, wait) = mpsc::channel();
        if tx.send(Message::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }
    }
}

fn nanos(t: SystemTime) -> String {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    let _ = getrandom::fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn attribute(key: &str, value: &Value) -> Value {
    let value = match value {
        Value::Bool(b) => json!({ "boolValue": b }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "intValue": n.to_string() }),
        Value::Number(n) => json!({ "doubleValue": n.as_f64() }),
        Value::String(s) => json!({ "stringValue": s }),
        other => json!({ "stringValue": other.to_string() }),
    };
    json!({ "key": key, "value": value })
}

/// The OTLP request of a batch of spans.
pub fn body(spans: &[Span], service: &str) -> Value {
    let spans: Vec<Value> = spans
        .iter()
        .map(|s| {
            let status = match &s.error {
                None => json!({ "code": 1 }),
                Some(message) => json!({ "code": 2, "message": message }),
            };
            json!({
                "traceId": random_hex(16),
                "spanId": random_hex(8),
                "name": s.name,
                "kind": 1,
                "startTimeUnixNano": nanos(s.start),
                "endTimeUnixNano": nanos(s.end),
                "attributes": s.attributes.iter().map(|(k, v)| attribute(k, v)).collect::<Vec<_>>(),
                "status": status,
            })
        })
        .collect();
    json!({
        "resourceSpans": [{
            "resource": { "attributes": [
                attribute("service.name", &json!(service)),
                attribute("service.version", &json!(env!("CARGO_PKG_VERSION"))),
            ]},
            "scopeSpans": [{ "scope": { "name": "apeiron" }, "spans": spans }],
        }]
    })
}

/// `OTEL_EXPORTER_OTLP_HEADERS`: `key=value` pairs separated by commas.
fn headers() -> Vec<(String, String)> {
    std::env::var("OTEL_EXPORTER_OTLP_HEADERS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

fn export(endpoint: &str, rx: &Receiver<Message>) {
    let service = std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "apeiron".to_owned());
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return;
    };
    let headers = headers();
    let mut batch = Vec::new();
    let send = |batch: &mut Vec<Span>| {
        if batch.is_empty() {
            return;
        }
        let mut request = client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .body(body(batch, &service).to_string());
        for (k, v) in &headers {
            request = request.header(k, v);
        }
        if let Err(e) = request.send().and_then(|r| r.error_for_status()) {
            eprintln!("OpenTelemetry: {e}");
        }
        batch.clear();
    };
    // A batch goes out after two quiet seconds, once it holds 100 spans,
    // or when asked to flush.
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Message::Span(span)) => {
                batch.push(span);
                if batch.len() >= 100 {
                    send(&mut batch);
                }
            }
            Ok(Message::Flush(done)) => {
                send(&mut batch);
                let _ = done.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => send(&mut batch),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                send(&mut batch);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_become_an_otlp_request() {
        let start = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let spans = [Span {
            name: "job documents".to_owned(),
            start,
            end: start + Duration::from_millis(1500),
            error: Some("no such folder".to_owned()),
            attributes: vec![
                ("apeiron.job".to_owned(), json!("documents")),
                ("apeiron.files".to_owned(), json!(12)),
            ],
        }];
        let b = body(&spans, "apeiron");
        let span = &b["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(span["name"], "job documents");
        assert_eq!(span["startTimeUnixNano"], "1700000000000000000");
        assert_eq!(span["endTimeUnixNano"], "1700000001500000000");
        assert_eq!(span["status"]["code"], 2);
        assert_eq!(span["traceId"].as_str().unwrap().len(), 32);
        assert_eq!(span["spanId"].as_str().unwrap().len(), 16);
        assert_eq!(span["attributes"][1]["value"]["intValue"], "12");
        assert_eq!(
            b["resourceSpans"][0]["resource"]["attributes"][0]["value"]["stringValue"],
            "apeiron"
        );
    }
}
