//! Searches saved under a name (SEARCH.md §6). They belong to the user, not
//! to a repository; `file` is the file holding them, the user's by default.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::SearchRequest;
use crate::error::{Error, ErrorKind, Result};

/// Longest name of a saved search, in characters.
const NAME_CHARS: usize = 64;

/// A search kept under a name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    #[serde(flatten)]
    pub request: SearchRequest,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    version: u32,
    searches: BTreeMap<String, SearchRequest>,
}

/// The file used when none is given: `APEIRON_SEARCHES`, or `searches.json`
/// in the user's Apeiron configuration folder.
pub fn searches_file() -> Result<PathBuf> {
    if let Some(file) = std::env::var_os("APEIRON_SEARCHES") {
        return Ok(PathBuf::from(file));
    }
    dirs::config_dir()
        .map(|c| c.join("apeiron").join("searches.json"))
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "no configuration folder"))
}

fn io(file: &Path, err: std::io::Error) -> Error {
    let kind = match err.kind() {
        std::io::ErrorKind::NotFound => ErrorKind::NotFound,
        _ => ErrorKind::Io,
    };
    Error::new(kind, format!("{}: {err}", file.display()))
}

fn load(file: &Path) -> Result<Stored> {
    let text = match fs::read(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Stored::default()),
        Err(e) => return Err(io(file, e)),
    };
    serde_json::from_slice(&text).map_err(|e| {
        Error::new(
            ErrorKind::Corrupt,
            format!("{}: not a file of saved searches: {e}", file.display()),
        )
    })
}

/// Writes the file whole: to a temporary file, synced, then renamed over it.
fn store(file: &Path, stored: &Stored) -> Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let text = serde_json::to_vec_pretty(stored)
        .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    let tmp = file.with_extension("json.tmp");
    let mut out = fs::File::create(&tmp).map_err(|e| io(&tmp, e))?;
    out.write_all(&text)
        .and_then(|()| out.sync_all())
        .map_err(|e| io(&tmp, e))?;
    fs::rename(&tmp, file).map_err(|e| io(file, e))
}

fn path(file: Option<&Path>) -> Result<PathBuf> {
    match file {
        Some(file) => Ok(file.to_path_buf()),
        None => searches_file(),
    }
}

/// The saved searches, by name.
pub fn saved_searches(file: Option<&Path>) -> Result<Vec<SavedSearch>> {
    Ok(load(&path(file)?)?
        .searches
        .into_iter()
        .map(|(name, request)| SavedSearch { name, request })
        .collect())
}

/// Saves a search under a name, replacing one of the same name. The search
/// is not run; a search with no criterion is `invalid_input`.
pub fn save_search(saved: &SavedSearch, file: Option<&Path>) -> Result<()> {
    let name = saved.name.trim();
    if name.is_empty() || name.chars().count() > NAME_CHARS || name.chars().any(char::is_control) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("a search name has 1 to {NAME_CHARS} characters and no control characters"),
        ));
    }
    let q = &saved.request.query;
    if q.pattern.is_none()
        && q.text.is_none()
        && q.larger.is_none()
        && q.smaller.is_none()
        && q.after.is_none()
        && q.before.is_none()
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "a search needs at least one criterion",
        ));
    }
    let file = path(file)?;
    let mut stored = load(&file)?;
    stored.version = 1;
    stored
        .searches
        .insert(name.to_owned(), saved.request.clone());
    store(&file, &stored)
}

/// The saved search with this name; `not_found` when there is none.
pub fn saved_search(name: &str, file: Option<&Path>) -> Result<SavedSearch> {
    let file = path(file)?;
    let request = load(&file)?.searches.remove(name.trim()).ok_or_else(|| {
        Error::new(
            ErrorKind::NotFound,
            format!("no saved search named {name:?}"),
        )
    })?;
    Ok(SavedSearch {
        name: name.trim().to_owned(),
        request,
    })
}

/// Removes a saved search and returns it; `not_found` when there is none.
pub fn forget_search(name: &str, file: Option<&Path>) -> Result<SavedSearch> {
    let file = path(file)?;
    let mut stored = load(&file)?;
    let request = stored.searches.remove(name.trim()).ok_or_else(|| {
        Error::new(
            ErrorKind::NotFound,
            format!("no saved search named {name:?}"),
        )
    })?;
    store(&file, &stored)?;
    Ok(SavedSearch {
        name: name.trim().to_owned(),
        request,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Query;

    fn request(pattern: &str) -> SearchRequest {
        SearchRequest {
            snapshot: None,
            history: true,
            query: Query {
                pattern: Some(pattern.to_owned()),
                ..Query::default()
            },
            limit: 10,
        }
    }

    #[test]
    fn searches_are_saved_replaced_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cfg").join("searches.json");
        let f = Some(file.as_path());
        assert!(saved_searches(f).unwrap().is_empty());

        let photos = SavedSearch {
            name: " photos ".to_owned(),
            request: request("*.jpg"),
        };
        save_search(&photos, f).unwrap();
        save_search(
            &SavedSearch {
                name: "docs".to_owned(),
                request: request("*.pdf"),
            },
            f,
        )
        .unwrap();
        let names: Vec<String> = saved_searches(f)
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["docs", "photos"]);

        save_search(
            &SavedSearch {
                name: "photos".to_owned(),
                request: request("*.png"),
            },
            f,
        )
        .unwrap();
        let got = saved_search("photos", f).unwrap();
        assert_eq!(got.request.query.pattern.as_deref(), Some("*.png"));
        assert!(got.request.history);

        assert_eq!(forget_search("photos", f).unwrap().name, "photos");
        assert_eq!(
            saved_search("photos", f).unwrap_err().kind,
            ErrorKind::NotFound
        );
        assert_eq!(
            forget_search("photos", f).unwrap_err().kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn bad_names_and_empty_searches_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("searches.json");
        let f = Some(file.as_path());
        let mut bad = SavedSearch {
            name: "  ".to_owned(),
            request: request("*"),
        };
        assert_eq!(
            save_search(&bad, f).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
        bad.name = "x".repeat(65);
        assert_eq!(
            save_search(&bad, f).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
        bad.name = "empty".to_owned();
        bad.request.query = Query::default();
        assert_eq!(
            save_search(&bad, f).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
        assert!(!file.exists());

        fs::write(&file, b"not json").unwrap();
        assert_eq!(saved_searches(f).unwrap_err().kind, ErrorKind::Corrupt);
    }
}
