//! Keeps the documents in step with each other and with the program
//! (CONTRIBUTING.md §7). The rules are in the test names.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every Markdown file of the project, relative to its root. `CLAUDE.md` is the
/// one working note kept in a single language.
fn markdown_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    walk(&path, found);
                }
            } else if name.ends_with(".md") && name != "CLAUDE.md" {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&root(), &mut found);
    found.sort();
    found
}

fn is_turkish(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".tr.md")
}

fn turkish_of(path: &Path) -> PathBuf {
    path.with_extension("tr.md")
}

/// The lines outside fenced code blocks.
fn prose(text: &str) -> Vec<&str> {
    let mut code = false;
    text.lines()
        .filter(|line| {
            if line.starts_with("```") {
                code = !code;
                false
            } else {
                !code
            }
        })
        .collect()
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    let rest = line.get(level..)?.strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, rest))
}

/// The address GitHub gives a heading.
fn anchor(title: &str) -> String {
    title
        .replace(['`', '*'], "")
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    let mut all = BTreeSet::new();
    for (_, title) in prose(text).into_iter().filter_map(heading) {
        let name = anchor(title);
        let count = seen.entry(name.clone()).or_default();
        all.insert(match *count {
            0 => name,
            n => format!("{name}-{n}"),
        });
        *count += 1;
    }
    all
}

/// The numbers of a document's numbered sections, such as `15` and `15.8`.
fn sections(text: &str) -> BTreeSet<String> {
    prose(text)
        .into_iter()
        .filter_map(heading)
        .filter_map(|(_, title)| {
            let number = title.split_whitespace().next()?.trim_end_matches('.');
            let numeric = number.split('.').all(|p| p.parse::<u32>().is_ok());
            numeric.then(|| number.to_string())
        })
        .collect()
}

/// The targets of the Markdown links on a line.
fn links(line: &str) -> Vec<&str> {
    line.match_indices("](")
        .filter_map(|(at, _)| {
            let rest = &line[at + 2..];
            let end = rest.find(')')?;
            let target = &rest[..end];
            (!target.contains(char::is_whitespace)).then_some(target)
        })
        .collect()
}

/// Removes `.` and `..` from a path, without asking the file system.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[test]
fn every_document_has_both_languages_and_the_same_structure() {
    let mut problems = Vec::new();
    for english in markdown_files().iter().filter(|p| !is_turkish(p)) {
        let turkish = turkish_of(english);
        if !turkish.exists() {
            problems.push(format!("{}: no Turkish version", english.display()));
            continue;
        }
        let (en, tr) = (read(english), read(&turkish));
        let shape = |text: &str| {
            let headings: Vec<usize> = prose(text)
                .into_iter()
                .filter_map(heading)
                .map(|(level, _)| level)
                .collect();
            let tables = prose(text).iter().filter(|l| l.starts_with('|')).count();
            let fences = text.lines().filter(|l| l.starts_with("```")).count();
            (headings, tables, fences)
        };
        if shape(&en) != shape(&tr) {
            problems.push(format!(
                "{}: headings, table rows or code blocks differ from the Turkish version",
                english.display()
            ));
        }
        let name = turkish.file_name().unwrap().to_string_lossy().into_owned();
        if !en.lines().next().unwrap_or("").contains(&name) {
            problems.push(format!(
                "{}: first line must link to {name}",
                english.display()
            ));
        }
        let name = english.file_name().unwrap().to_string_lossy().into_owned();
        if !tr.lines().next().unwrap_or("").contains(&name) {
            problems.push(format!(
                "{}: first line must link to {name}",
                turkish.display()
            ));
        }
    }
    for turkish in markdown_files().iter().filter(|p| is_turkish(p)) {
        let name = turkish.to_string_lossy().replace(".tr.md", ".md");
        if !Path::new(&name).exists() {
            problems.push(format!("{}: no English version", turkish.display()));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn links_and_anchors_lead_somewhere() {
    let mut problems = Vec::new();
    for file in markdown_files() {
        let text = read(&file);
        for line in prose(&text) {
            for target in links(line) {
                if target.starts_with("http") || target.starts_with("mailto:") {
                    continue;
                }
                let (path, fragment) = target.split_once('#').unwrap_or((target, ""));
                let dest = if path.is_empty() {
                    file.clone()
                } else {
                    normal(&file.parent().unwrap().join(path))
                };
                if !dest.exists() {
                    problems.push(format!("{}: no file {target}", file.display()));
                } else if !fragment.is_empty()
                    && dest.extension().is_some_and(|e| e == "md")
                    && !anchors(&read(&dest)).contains(fragment)
                {
                    problems.push(format!("{}: no heading {target}", file.display()));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn section_references_name_sections_that_exist() {
    let docs = root().join("docs");
    let mut problems = Vec::new();
    for file in markdown_files() {
        for line in read(&file).lines() {
            for (at, _) in line.match_indices('§') {
                let number: String = line[at + '§'.len_utf8()..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                let number = number.trim_end_matches('.');
                // The word before the mark: `FORMAT.md` or a link to it.
                let before = line[..at].trim_end();
                let before = match before.strip_suffix(')') {
                    Some(linked) => linked.rsplit("](").nth(1).unwrap_or(linked),
                    None => before,
                };
                let word = before
                    .rsplit(|c: char| c.is_whitespace() || c == '(' || c == '[')
                    .next()
                    .unwrap_or("");
                let Some(name) = word.rsplit('/').next().filter(|w| w.ends_with(".md")) else {
                    continue;
                };
                let target = docs.join(name);
                if target.exists() && !sections(&read(&target)).contains(number) {
                    problems.push(format!("{}: {name} §{number}", file.display()));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The options and the subcommands `--help` shows for a command.
fn help(path: &[String]) -> (Vec<String>, Vec<String>) {
    let out = Command::new(env!("CARGO_BIN_EXE_apeiron"))
        .args(path)
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success(), "apeiron {path:?} --help");
    let text = String::from_utf8(out.stdout).unwrap();
    let mut options = Vec::new();
    let mut commands = Vec::new();
    let mut in_commands = false;
    for line in text.lines() {
        if line == "Commands:" {
            in_commands = true;
        } else if line.is_empty() || !line.starts_with(' ') {
            in_commands = false;
        } else if in_commands {
            // A wrapped description is indented further than a command's name.
            let name = line.split_whitespace().next().unwrap_or("");
            if name != "help" && line.len() - line.trim_start().len() == 2 {
                commands.push(name.to_string());
            }
        } else {
            let trimmed = line.trim_start();
            let trimmed = match trimmed.split_once(", ") {
                Some((short, rest)) if short.len() == 2 && short.starts_with('-') => rest,
                _ => trimmed,
            };
            if trimmed.starts_with("--") {
                let flag = trimmed.split([' ', '=']).next().unwrap_or("");
                options.push(flag.to_string());
            }
        }
    }
    (options, commands)
}

#[test]
fn every_command_and_option_of_the_program_is_documented() {
    let root = root();
    let cli = read(&root.join("docs/CLI.md"));
    let all: String = fs::read_dir(root.join("docs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md") && !is_turkish(p))
        .map(|p| read(&p))
        .collect();

    let mut problems = BTreeSet::new();
    let mut queue = vec![Vec::<String>::new()];
    let mut walked = 0;
    while let Some(path) = queue.pop() {
        walked += 1;
        let (options, commands) = help(&path);
        for option in options {
            if option != "--help" && option != "--version" && !all.contains(&option) {
                problems.insert(format!("option {option} of `apeiron {}`", path.join(" ")));
            }
        }
        for command in commands {
            let mut next = path.clone();
            next.push(command.clone());
            if !cli.contains(&format!(" {command}")) && !cli.contains(&format!("`{command}")) {
                problems.insert(format!(
                    "command `apeiron {}` is not in CLI.md",
                    next.join(" ")
                ));
            }
            queue.push(next);
        }
    }
    assert!(walked > 50, "only {walked} commands found");
    let list: Vec<String> = problems.into_iter().collect();
    assert!(list.is_empty(), "{}", list.join("\n"));
}

#[test]
fn every_operation_of_the_c_abi_is_in_the_sdk_document() {
    let root = root();
    let source = read(&root.join("crates/apeiron/src/json.rs"));
    let sdk = read(&root.join("docs/SDK.md"));
    let mut operations = 0;
    let mut missing = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some((name, tail)) = rest.split_once('"') else {
            continue;
        };
        if tail.trim_start().starts_with("=>") {
            operations += 1;
            if !sdk.contains(&format!("`{name}`")) {
                missing.push(name.to_string());
            }
        }
    }
    assert!(operations > 30, "only {operations} operations found");
    assert!(missing.is_empty(), "not in SDK.md: {missing:?}");
}

#[test]
fn the_documents_name_every_crate_and_the_minimum_rust_version() {
    let root = root();
    let readme = read(&root.join("README.md"));
    for entry in fs::read_dir(root.join("crates")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            readme.contains(&format!("`{name}`")),
            "{name} not in README"
        );
    }
    let manifest = read(&root.join("Cargo.toml"));
    let version = manifest
        .lines()
        .find_map(|l| l.strip_prefix("rust-version = "))
        .unwrap()
        .trim_matches('"');
    for file in [
        "README.md",
        "README.tr.md",
        "CONTRIBUTING.md",
        "CONTRIBUTING.tr.md",
    ] {
        assert!(
            read(&root.join(file)).contains(version),
            "{file} does not name Rust {version}"
        );
    }
}

#[test]
fn the_readme_and_the_roadmap_give_every_stage_the_same_status() {
    let root = root();
    // `| 3. Name | … | Done |`: the number of the stage and its last column.
    let stages = |text: &str| -> BTreeMap<char, String> {
        let mut found = BTreeMap::new();
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("| ") else {
                continue;
            };
            let mut chars = rest.chars();
            let (Some(number), Some('.')) = (chars.next(), chars.next()) else {
                continue;
            };
            let columns: Vec<&str> = line.trim_matches('|').split(" | ").collect();
            if number.is_ascii_digit() {
                let status = columns.last().unwrap().trim();
                // The roadmap may add a note in parentheses.
                let word = status.split(" (").next().unwrap().to_string();
                found.insert(number, word);
            }
        }
        found
    };
    for (readme, roadmap) in [
        ("README.md", "docs/ROADMAP.md"),
        ("README.tr.md", "docs/ROADMAP.tr.md"),
    ] {
        let a = stages(&read(&root.join(readme)));
        let b = stages(&read(&root.join(roadmap)));
        assert_eq!(a.len(), 6, "{readme}");
        assert_eq!(a, b, "{readme} and {roadmap}");
    }
}
