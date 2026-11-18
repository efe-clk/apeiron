//! Sizes, times and values of results, written as the pages write them.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;

/// A size in B, KB, MB, GB or TB, with one decimal under 100.
pub fn bytes(n: u64, lang: &str) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = if unit == 0 || value >= 100.0 {
        format!("{value:.0}")
    } else {
        let text = format!("{value:.1}");
        text.strip_suffix(".0").map(str::to_owned).unwrap_or(text)
    };
    let text = if lang == "tr" {
        text.replace('.', ",")
    } else {
        text
    };
    format!("{text} {}", UNITS[unit])
}

/// A Unix time in seconds as local date and time.
pub fn seconds(s: i64) -> String {
    Timestamp::from_second(s)
        .map(|t| {
            t.to_zoned(TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// The local date of a Unix time, `YYYY-MM-DD`.
pub fn date(s: i64) -> String {
    Timestamp::from_second(s)
        .map(|t| {
            t.to_zoned(TimeZone::system())
                .strftime("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

/// A time of the API, `{"seconds": …, "nanos": …}`, as local date and time;
/// empty when absent.
pub fn when(time: &Value) -> String {
    time.get("seconds")
        .and_then(Value::as_i64)
        .map(seconds)
        .unwrap_or_default()
}

/// The seconds of a time of the API.
pub fn secs(time: &Value) -> i64 {
    time.get("seconds").and_then(Value::as_i64).unwrap_or(0)
}

/// A string field of a result; empty when absent.
pub fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.clone(),
        // A name that is not UTF-8 (FORMAT.md §6.1).
        Some(Value::Object(o)) => o
            .get("hex")
            .and_then(Value::as_str)
            .map(|h| format!("<{h}>"))
            .unwrap_or_default(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// A number field of a result; 0 when absent.
pub fn num(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// An array field of a result; empty when absent.
pub fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_written_like_the_pages() {
        assert_eq!(bytes(17, "en"), "17 B");
        assert_eq!(bytes(1536, "en"), "1.5 KB");
        assert_eq!(bytes(1536, "tr"), "1,5 KB");
        assert_eq!(bytes(1024 * 1024, "en"), "1 MB");
        assert_eq!(bytes(200 * 1024, "en"), "200 KB");
    }

    #[test]
    fn names_that_are_not_text_show_their_bytes() {
        let v = serde_json::json!({"a": "x", "b": {"hex": "ff"}, "c": null});
        assert_eq!(text(&v, "a"), "x");
        assert_eq!(text(&v, "b"), "<ff>");
        assert_eq!(text(&v, "c"), "");
    }
}
