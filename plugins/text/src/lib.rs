//! Merges text files line by line and shows their differences (TEXT.md).
//! A plugin of the interface `apeiron:plugin/type-driver@0.1.0`.

wit_bindgen::generate!({ path: "../../crates/apeiron-plugin/wit", world: "type-driver-plugin" });

use exports::apeiron::plugin::type_driver::{Guest, Info};

/// The files this plugin handles (TEXT.md §1).
pub const PATTERNS: &[&str] = &[
    "*.txt",
    "*.md",
    "*.markdown",
    "*.rst",
    "*.adoc",
    "*.tex",
    "*.csv",
    "*.tsv",
    "*.json",
    "*.toml",
    "*.yaml",
    "*.yml",
    "*.ini",
    "*.cfg",
    "*.conf",
    "*.xml",
    "*.svg",
    "*.html",
    "*.htm",
    "*.css",
    "*.js",
    "*.ts",
    "*.jsx",
    "*.tsx",
    "*.py",
    "*.rb",
    "*.rs",
    "*.go",
    "*.c",
    "*.h",
    "*.cc",
    "*.cpp",
    "*.hpp",
    "*.java",
    "*.kt",
    "*.swift",
    "*.cs",
    "*.php",
    "*.sh",
    "*.bash",
    "*.zsh",
    "*.ps1",
    "*.sql",
    "*.lua",
    "*.pl",
    "*.r",
    "*.m",
    "*.el",
    "*.org",
    "*.srt",
    "*.vtt",
    "*.log",
];

/// Text a plugin can work on: valid UTF-8 without a zero byte.
fn text(data: &[u8]) -> Option<&str> {
    if data.contains(&0) {
        return None;
    }
    std::str::from_utf8(data).ok()
}

/// Joins the changes of both sides since `base` (TEXT.md §2): the merged
/// text, or `None` when they touch the same lines, or a file is not text.
pub fn merge(base: Option<&[u8]>, ours: &[u8], theirs: &[u8]) -> Option<Vec<u8>> {
    let (base, ours, theirs) = (text(base.unwrap_or_default())?, text(ours)?, text(theirs)?);
    match diffy::merge(base, ours, theirs) {
        Ok(merged) => Some(merged.into_bytes()),
        Err(_) => merge_json(base, ours, theirs).or_else(|| merge_markdown(base, ours, theirs)),
    }
}

/// Splits a Markdown text at its headings (lines of one to six `#` and a
/// space, outside fenced code): the text before the first heading under the
/// key `""`, then each section under its heading line. `None` when two
/// headings are the same, so sections cannot be told apart.
fn sections(text: &str) -> Option<Vec<(&str, &str)>> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    let (mut key, mut start, mut at) = ("", 0, 0);
    let mut fence: Option<&str> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        let marker = ["```", "~~~"]
            .into_iter()
            .find(|m| trimmed.trim_start().starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => fence = Some(m),
            (Some(f), Some(m)) if f == m => fence = None,
            _ => {}
        }
        let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
        let heading = fence.is_none()
            && marker.is_none()
            && (1..=6).contains(&hashes)
            && (trimmed.len() == hashes || trimmed[hashes..].starts_with(' '));
        if heading {
            out.push((key, &text[start..at]));
            (key, start) = (trimmed, at);
        }
        at += line.len();
    }
    out.push((key, &text[start..]));
    let mut keys: Vec<&str> = out.iter().map(|(k, _)| *k).collect();
    keys.sort_unstable();
    let before = keys.len();
    keys.dedup();
    (keys.len() == before).then_some(out)
}

/// Joins two changed Markdown texts section by section when their lines
/// collide (TEXT.md §2): a section changed on one side only is taken from
/// that side, one changed on both is merged line by line, and a new section
/// follows the section it follows on its side.
fn merge_markdown(base: &str, ours: &str, theirs: &str) -> Option<Vec<u8>> {
    let (b, o, t) = (sections(base)?, sections(ours)?, sections(theirs)?);
    if o.len() < 2 || t.len() < 2 {
        return None;
    }
    fn find<'a>(s: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
        s.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
    let mut out: Vec<(&str, String)> = Vec::new();
    for (key, ours_text) in &o {
        let merged = match (find(&b, key), find(&t, key)) {
            (base_text, Some(theirs_text)) => {
                let base_text = base_text.unwrap_or_default();
                if ours_text == &theirs_text || base_text == theirs_text {
                    ours_text.to_string()
                } else if base_text == *ours_text {
                    theirs_text.to_owned()
                } else {
                    diffy::merge(base_text, ours_text, theirs_text).ok()?
                }
            }
            // Removed there: fine if unchanged here.
            (Some(base_text), None) if base_text == *ours_text => continue,
            (Some(_), None) => return None,
            (None, None) => ours_text.to_string(),
        };
        out.push((key, merged));
    }
    for (i, (key, theirs_text)) in t.iter().enumerate() {
        if find(&o, key).is_some() {
            continue;
        }
        match find(&b, key) {
            // Removed here: fine if unchanged there.
            Some(base_text) if base_text == *theirs_text => {}
            Some(_) => return None,
            None => {
                let after = out.iter().position(|(k, _)| *k == t[i - 1].0);
                let at = after.map_or(out.len(), |p| p + 1);
                out.insert(at, (key, theirs_text.to_string()));
            }
        }
    }
    let mut text = String::new();
    for (_, section) in out {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&section);
    }
    Some(text.into_bytes())
}

/// Joins two changed JSON documents key by key when their lines collide
/// (TEXT.md §2): objects are merged member by member, anything else must be
/// changed on one side only. The result keeps the members in the order of
/// `ours`, then the new ones of `theirs`, and the indentation of `ours`.
fn merge_json(base: &str, ours: &str, theirs: &str) -> Option<Vec<u8>> {
    use serde_json::Value;
    let parse = |t: &str| serde_json::from_str::<Value>(t).ok();
    let base = if base.trim().is_empty() {
        None
    } else {
        Some(parse(base)?)
    };
    let merged = merge_values(base.as_ref(), &parse(ours)?, &parse(theirs)?)?;
    let indent = ours
        .lines()
        .nth(1)
        .map(|l| &l[..l.len() - l.trim_start().len()])
        .filter(|i| !i.is_empty())
        .unwrap_or("  ");
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut writer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(&merged, &mut writer).ok()?;
    if ours.ends_with('\n') {
        out.push(b'\n');
    }
    Some(out)
}

/// A three-way merge of JSON values; `None` when both sides changed the
/// same value differently.
fn merge_values(
    base: Option<&serde_json::Value>,
    ours: &serde_json::Value,
    theirs: &serde_json::Value,
) -> Option<serde_json::Value> {
    use serde_json::Value;
    if ours == theirs || base == Some(theirs) {
        return Some(ours.clone());
    }
    if base == Some(ours) {
        return Some(theirs.clone());
    }
    let (Value::Object(o), Value::Object(t)) = (ours, theirs) else {
        return None;
    };
    let b = match base {
        Some(Value::Object(b)) => Some(b),
        None => None,
        Some(_) => return None,
    };
    let in_base = |k: &str| b.and_then(|b| b.get(k));
    let mut out = serde_json::Map::new();
    for (key, ours_value) in o {
        match t.get(key) {
            Some(theirs_value) => {
                out.insert(
                    key.clone(),
                    merge_values(in_base(key), ours_value, theirs_value)?,
                );
            }
            // Removed there: fine if unchanged here, a conflict otherwise.
            None => match in_base(key) {
                Some(base_value) if base_value == ours_value => {}
                Some(_) => return None,
                None => {
                    out.insert(key.clone(), ours_value.clone());
                }
            },
        }
    }
    for (key, theirs_value) in t {
        if o.contains_key(key) {
            continue;
        }
        match in_base(key) {
            // Removed here: fine if unchanged there.
            Some(base_value) if base_value == theirs_value => {}
            Some(_) => return None,
            None => {
                out.insert(key.clone(), theirs_value.clone());
            }
        }
    }
    Some(Value::Object(out))
}

/// The changed lines of a text file with three lines around each change,
/// as in `diff -u` without its first two lines; `None` when nothing
/// changed or a file is not text (TEXT.md §3).
pub fn diff(old: &[u8], new: &[u8]) -> Option<String> {
    let (old, new) = (text(old)?, text(new)?);
    if old == new {
        return None;
    }
    let patch = diffy::create_patch(old, new).to_string();
    // The header names the files "original" and "modified"; the caller
    // knows their real names.
    Some(patch.lines().skip(2).map(|l| format!("{l}\n")).collect())
}

// Exported only from WebAssembly: on other targets the export names of the
// component model are not valid symbol names, and only the tests run there.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct Text;

impl Guest for Text {
    fn describe() -> Info {
        Info {
            name: "text".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Merges text files line by line and shows their differences".into(),
            patterns: PATTERNS.iter().map(|p| (*p).to_owned()).collect(),
            permissions: Vec::new(),
        }
    }

    fn merge(
        base: Option<Vec<u8>>,
        ours: Vec<u8>,
        theirs: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, String> {
        Ok(merge(base.as_deref(), &ours, &theirs))
    }

    fn diff(old: Vec<u8>, new: Vec<u8>) -> Result<Option<String>, String> {
        Ok(diff(&old, &new))
    }
}

#[cfg(target_arch = "wasm32")]
export!(Text);

#[cfg(test)]
mod tests {
    use super::*;

    fn m(base: &str, ours: &str, theirs: &str) -> Option<String> {
        merge(Some(base.as_bytes()), ours.as_bytes(), theirs.as_bytes())
            .map(|v| String::from_utf8(v).unwrap())
    }

    #[test]
    fn changes_to_different_lines_are_joined() {
        let base = "one\ntwo\nthree\nfour\nfive\n";
        assert_eq!(
            m(
                base,
                "ONE\ntwo\nthree\nfour\nfive\n",
                "one\ntwo\nthree\nfour\nFIVE\n"
            )
            .as_deref(),
            Some("ONE\ntwo\nthree\nfour\nFIVE\n")
        );
        // Lines added at both ends.
        assert_eq!(
            m(
                base,
                "zero\none\ntwo\nthree\nfour\nfive\n",
                "one\ntwo\nthree\nfour\nfive\nsix\n"
            )
            .as_deref(),
            Some("zero\none\ntwo\nthree\nfour\nfive\nsix\n")
        );
    }

    #[test]
    fn the_same_change_on_both_sides_is_taken_once() {
        let base = "a\nb\nc\n";
        assert_eq!(
            m(base, "a\nB\nc\n", "a\nB\nc\n").as_deref(),
            Some("a\nB\nc\n")
        );
    }

    #[test]
    fn changes_to_the_same_lines_conflict() {
        let base = "a\nb\nc\n";
        assert_eq!(m(base, "a\nours\nc\n", "a\ntheirs\nc\n"), None);
        // Without a base, two new files conflict unless they are equal.
        assert_eq!(merge(None, b"x\n", b"y\n"), None);
    }

    #[test]
    fn json_members_changed_on_both_sides_are_joined_by_key() {
        let base = "{\n    \"name\": \"app\",\n    \"port\": 80\n}\n";
        let ours = "{\n    \"name\": \"app\",\n    \"port\": 8080,\n    \"debug\": true\n}\n";
        let theirs =
            "{\n    \"name\": \"web app\",\n    \"port\": 80,\n    \"tls\": {\"on\": true}\n}\n";
        assert_eq!(
            m(base, ours, theirs).as_deref(),
            Some(
                "{\n    \"name\": \"web app\",\n    \"port\": 8080,\n    \"debug\": true,\n    \"tls\": {\n        \"on\": true\n    }\n}\n"
            )
        );
        // Nested objects merge too; a member removed on one side stays removed.
        assert_eq!(
            m(
                r#"{"a": {"x": 1, "y": 1}, "gone": 1}"#,
                r#"{"a": {"x": 2, "y": 1}}"#,
                r#"{"a": {"x": 1, "y": 2}, "gone": 1}"#
            )
            .as_deref(),
            Some("{\n  \"a\": {\n    \"x\": 2,\n    \"y\": 2\n  }\n}")
        );
    }

    #[test]
    fn json_values_changed_differently_still_conflict() {
        let base = r#"{"port": 80}"#;
        assert_eq!(m(base, r#"{"port": 1}"#, r#"{"port": 2}"#), None);
        // Removed on one side, changed on the other.
        assert_eq!(
            m(r#"{"a": 1, "b": 1}"#, r#"{"b": 1}"#, r#"{"a": 2, "b": 1}"#),
            None
        );
        // Text that only looks like JSON on one side.
        assert_eq!(m("a\nb\n", "{}", "x\ny\n"), None);
    }

    #[test]
    fn markdown_sections_changed_on_both_sides_are_joined() {
        let base = "# Notes\n\n## Todo\n- milk\n## Done\n- bread\n";
        // Sections added at the same place on both sides are both kept, the
        // other side's first, and a section changed on both sides is merged
        // line by line.
        let ours = format!("{base}## Ideas\n- a\n").replace("- milk", "- oat milk");
        let theirs = format!("{base}## Later\n- b").replace("- bread", "- rye bread");
        assert!(diffy::merge(base, &ours, &theirs).is_err());
        assert_eq!(
            m(base, &ours, &theirs).as_deref(),
            Some(
                "# Notes\n\n## Todo\n- oat milk\n## Done\n- rye bread\n## Later\n- b\n## Ideas\n- a\n"
            )
        );
        let ours = format!("{base}## Ideas\n- a\n");
        let theirs = format!("{base}## Later\n- b\n").replace("# Notes\n\n", "# Notes\nshop\n\n");
        assert_eq!(
            m(base, &ours, &theirs).as_deref(),
            Some(
                "# Notes\nshop\n\n## Todo\n- milk\n## Done\n- bread\n## Later\n- b\n## Ideas\n- a\n"
            )
        );
        // A section removed on one side and left alone on the other is gone.
        assert_eq!(
            m(
                base,
                "# Notes\n\n## Todo\n- milk\n",
                &format!("{base}## Later\n")
            )
            .as_deref(),
            Some("# Notes\n\n## Todo\n- milk\n## Later\n")
        );
    }

    #[test]
    fn markdown_sections_changed_differently_still_conflict() {
        let base = "# A\none\n# B\ntwo\n";
        assert_eq!(
            m(
                base,
                "# A\nours\n# B\ntwo\n# C\n",
                "# A\ntheirs\n# B\ntwo\n# D\n"
            ),
            None
        );
        // Removed on one side, changed on the other.
        assert_eq!(
            m(base, "# A\none\n# C\n", "# A\none\n# B\nnew\n# D\n"),
            None
        );
        // A `#` line inside fenced code is not a heading, and two equal
        // headings cannot be told apart.
        assert_eq!(sections("```\n# x\n```\n# y\n").map(|s| s.len()), Some(2));
        assert_eq!(sections("# x\na\n# x\n"), None);
    }

    #[test]
    fn files_that_are_not_text_are_left_alone() {
        assert_eq!(merge(Some(b"a"), b"a\0b", b"a"), None);
        assert_eq!(merge(Some(b"a"), b"a", &[0xff, 0xfe]), None);
        assert_eq!(diff(b"\0", b"x"), None);
    }

    #[test]
    fn the_difference_shows_changed_lines_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n9\n";
        assert_eq!(
            diff(old.as_bytes(), new.as_bytes()).as_deref(),
            Some("@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+five\n 6\n 7\n 8\n")
        );
        assert_eq!(diff(b"same\n", b"same\n"), None);
        assert_eq!(
            diff(b"", b"new\n").as_deref(),
            Some("@@ -0,0 +1 @@\n+new\n")
        );
    }
}
