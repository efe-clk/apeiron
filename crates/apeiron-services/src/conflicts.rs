//! Finding and resolving the conflict copies sync and merge leave in a
//! folder (SYNC.md §4).

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};

/// A conflict copy and the file it stands beside, both relative to the
/// folder, with `/` between names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictCopy {
    pub path: String,
    pub copy: String,
    /// Whether the file at `path` still exists.
    pub exists: bool,
}

/// Which version of a conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    /// The file at the path; the copy is deleted.
    This,
    /// The copy, which replaces the file at the path.
    Copy,
}

/// The name a conflict copy stands beside: `report (conflict pc 2026-10-31
/// 10-08 2).txt` gives `report.txt`. `None` for any other name.
pub fn original_name(name: &str) -> Option<String> {
    let start = name.rfind(" (conflict ")?;
    let end = start + name[start..].find(')')?;
    let ext = &name[end + 1..];
    let one_extension = ext.is_empty() || (ext.starts_with('.') && !ext[1..].contains(['.', ')']));
    (start > 0 && end > start + " (conflict ".len() && one_extension)
        .then(|| format!("{}{ext}", &name[..start]))
}

/// The conflict copies in `folder` and the folders inside it, sorted by
/// path. Links to folders are not followed.
pub fn find_conflicts(folder: &Path) -> Result<Vec<ConflictCopy>> {
    let mut found = Vec::new();
    walk(folder, "", &mut found)?;
    found.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.copy.cmp(&b.copy)));
    Ok(found)
}

fn walk(dir: &Path, rel: &str, found: &mut Vec<ConflictCopy>) -> Result<()> {
    for entry in fs::read_dir(dir).at(dir)? {
        let entry = entry.at(dir)?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = format!("{rel}{name}");
        let kind = entry.file_type().at(&entry.path())?;
        if kind.is_dir() {
            walk(&entry.path(), &format!("{path}/"), found)?;
        } else if let Some(original) = original_name(&name) {
            found.push(ConflictCopy {
                exists: dir.join(&original).symlink_metadata().is_ok(),
                path: format!("{rel}{original}"),
                copy: path,
            });
        }
    }
    Ok(())
}

/// Keeps one version of the conflict copy `copy`, a path relative to
/// `folder`: deletes the copy, or puts it in place of the file it stands
/// beside. The next sync round takes the choice to the other devices.
pub fn resolve_conflict(folder: &Path, copy: &str, keep: Keep) -> Result<ConflictCopy> {
    let rel = Path::new(copy);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(Error::InvalidInput(format!(
            "{copy:?} is not a path inside the folder"
        )));
    }
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let original = original_name(name)
        .ok_or_else(|| Error::InvalidInput(format!("{copy:?} is not a conflict copy")))?;
    let copy_path = folder.join(rel);
    let kind = copy_path
        .symlink_metadata()
        .map_err(|_| Error::NotFound(copy.to_owned()))?;
    if kind.is_dir() {
        return Err(Error::InvalidInput(format!(
            "{copy:?} is a folder; move or delete what is in it by hand"
        )));
    }
    let original_path: PathBuf = copy_path.with_file_name(&original);
    match keep {
        Keep::This => fs::remove_file(&copy_path).at(&copy_path)?,
        Keep::Copy => {
            if original_path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                return Err(Error::InvalidInput(format!(
                    "{original:?} is a folder; move or delete it by hand"
                )));
            }
            // Replaces a file atomically on every system.
            fs::rename(&copy_path, &original_path).at(&copy_path)?;
        }
    }
    let parent = copy.rsplit_once('/').map_or("", |(p, _)| p);
    Ok(ConflictCopy {
        path: if parent.is_empty() {
            original
        } else {
            format!("{parent}/{original}")
        },
        copy: copy.to_owned(),
        exists: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_names() {
        let cases = [
            ("note (conflict pc 2026-10-31 10-08).txt", Some("note.txt")),
            (
                "note (conflict pc 2026-10-31 10-08 2).txt",
                Some("note.txt"),
            ),
            (
                "archive.tar (conflict pc 2026-10-31 10-08).gz",
                Some("archive.tar.gz"),
            ),
            ("Makefile (conflict backup-laptop)", Some("Makefile")),
            ("note.txt", None),
            (" (conflict pc).txt", None),
            ("note (conflict ).txt", None),
            ("note (conflict pc).tar.gz", None),
            ("note (conflict pc", None),
        ];
        for (name, original) in cases {
            assert_eq!(original_name(name).as_deref(), original, "{name}");
        }
    }

    #[test]
    fn finds_and_resolves_conflict_copies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("docs")).unwrap();
        fs::write(root.join("a.txt"), "here").unwrap();
        fs::write(root.join("a (conflict pc 2026-10-31 10-08).txt"), "there").unwrap();
        fs::write(root.join("docs/b.txt"), "here").unwrap();
        fs::write(
            root.join("docs/b (conflict pc 2026-10-31 10-08).txt"),
            "there",
        )
        .unwrap();
        fs::write(root.join("gone (conflict pc 2026-10-31 10-08)"), "there").unwrap();

        let found = find_conflicts(root).unwrap();
        let pairs: Vec<_> = found
            .iter()
            .map(|c| (c.path.as_str(), c.copy.as_str(), c.exists))
            .collect();
        assert_eq!(
            pairs,
            [
                ("a.txt", "a (conflict pc 2026-10-31 10-08).txt", true),
                (
                    "docs/b.txt",
                    "docs/b (conflict pc 2026-10-31 10-08).txt",
                    true
                ),
                ("gone", "gone (conflict pc 2026-10-31 10-08)", false),
            ]
        );

        let kept = resolve_conflict(root, &found[0].copy, Keep::This).unwrap();
        assert_eq!(kept.path, "a.txt");
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "here");
        let kept = resolve_conflict(root, &found[1].copy, Keep::Copy).unwrap();
        assert_eq!(kept.path, "docs/b.txt");
        assert_eq!(
            fs::read_to_string(root.join("docs/b.txt")).unwrap(),
            "there"
        );
        resolve_conflict(root, &found[2].copy, Keep::Copy).unwrap();
        assert_eq!(fs::read_to_string(root.join("gone")).unwrap(), "there");
        assert_eq!(find_conflicts(root).unwrap(), []);
    }

    #[test]
    fn refuses_what_is_not_a_conflict_copy_inside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "here").unwrap();
        for copy in ["a.txt", "../a (conflict pc).txt", "/a (conflict pc).txt"] {
            assert!(
                matches!(
                    resolve_conflict(root, copy, Keep::This),
                    Err(Error::InvalidInput(_))
                ),
                "{copy}"
            );
        }
        assert!(matches!(
            resolve_conflict(root, "b (conflict pc).txt", Keep::This),
            Err(Error::NotFound(_))
        ));
        fs::create_dir(root.join("d (conflict pc)")).unwrap();
        assert!(matches!(
            resolve_conflict(root, "d (conflict pc)", Keep::This),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "here");
    }
}
