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
    let base = text(base.unwrap_or_default())?;
    let merged = diffy::merge(base, text(ours)?, text(theirs)?).ok()?;
    Some(merged.into_bytes())
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
