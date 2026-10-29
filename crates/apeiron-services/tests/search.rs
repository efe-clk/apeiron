//! Searching snapshots and their history (SEARCH.md).

use std::fs;
use std::path::PathBuf;

use apeiron_core::{ObjectId, Repository, Time};
use apeiron_services::{
    BackupOptions, Control, Error, Query, SearchOptions, SearchOutcome, backup, search,
};

struct Setup {
    _dir: tempfile::TempDir,
    folder: PathBuf,
    repo: Repository,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(&dir.path().join("repo")).unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    Setup {
        _dir: dir,
        folder,
        repo,
    }
}

impl Setup {
    fn write(&self, name: &str, data: &[u8]) {
        let path = self.folder.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    fn backup(&mut self) -> ObjectId {
        let options = BackupOptions::new(&self.folder, "main");
        backup(&mut self.repo, &options, &Control::new())
            .unwrap()
            .snapshot
    }

    fn search(&self, query: Query, history: bool) -> SearchOutcome {
        let options = SearchOptions {
            starts: vec![self.repo.read_ref("main").unwrap().unwrap()],
            history,
            query,
            limit: 100,
        };
        search(&self.repo, &options, &Control::new()).unwrap()
    }
}

fn pattern(p: &str) -> Query {
    Query {
        pattern: Some(p.to_owned()),
        ..Query::default()
    }
}

fn paths(out: &SearchOutcome) -> Vec<String> {
    out.hits.iter().map(|h| h.path.to_string()).collect()
}

#[test]
fn names_and_paths_match_ignoring_case() {
    let mut s = setup();
    s.write("Notes.TXT", b"n");
    s.write("docs/a.txt", b"a");
    s.write("docs/deep/b.txt", b"b");
    s.write("docs/c.md", b"c");
    s.backup();

    assert_eq!(
        paths(&s.search(pattern("*.txt"), false)),
        ["Notes.TXT", "docs/a.txt", "docs/deep/b.txt"]
    );
    assert_eq!(paths(&s.search(pattern("DOCS"), false)), ["docs"]);
    assert_eq!(
        paths(&s.search(pattern("docs/*.txt"), false)),
        ["docs/a.txt", "docs/deep/b.txt"]
    );
    assert_eq!(paths(&s.search(pattern("?.md"), false)), ["docs/c.md"]);
    assert!(s.search(pattern("missing*"), false).hits.is_empty());
}

#[test]
fn history_lists_each_version_once_with_its_newest_snapshot() {
    let mut s = setup();
    s.write("keep.txt", b"same");
    s.write("change.txt", b"one");
    s.write("gone.txt", b"bye");
    let first = s.backup();
    s.write("change.txt", b"two");
    fs::remove_file(s.folder.join("gone.txt")).unwrap();
    s.backup();
    s.write("other.bin", b"x");
    let third = s.backup();

    let now = s.search(pattern("*.txt"), false);
    assert_eq!(paths(&now), ["change.txt", "keep.txt"]);
    assert!(now.hits.iter().all(|h| h.snapshot == third));
    assert_eq!(now.snapshots, 1);

    let all = s.search(pattern("*.txt"), true);
    let found: Vec<(String, ObjectId)> = all
        .hits
        .iter()
        .map(|h| (h.path.to_string(), h.snapshot))
        .collect();
    assert_eq!(
        found,
        [
            ("change.txt".to_owned(), third),
            ("keep.txt".to_owned(), third),
            ("change.txt".to_owned(), first),
            ("gone.txt".to_owned(), first),
        ]
    );
    assert_eq!(all.snapshots, 3);
}

#[test]
fn text_is_found_in_text_files_only() {
    let mut s = setup();
    s.write("a.txt", b"hello\nThe Quick fox\n");
    s.write("b.txt", b"nothing here\n");
    s.write("bin.dat", b"quick\0binary");
    s.write("copy/a.txt", b"hello\nThe Quick fox\n");
    s.write("empty.txt", b"");
    s.backup();

    let out = s.search(
        Query {
            text: Some("QUICK".to_owned()),
            ..Query::default()
        },
        false,
    );
    assert_eq!(paths(&out), ["a.txt", "copy/a.txt"]);
    let line = out.hits[0].line.as_ref().unwrap();
    assert_eq!((line.number, line.text.as_str()), (2, "The Quick fox"));

    let both = s.search(
        Query {
            pattern: Some("copy/*".to_owned()),
            text: Some("fox".to_owned()),
            ..Query::default()
        },
        false,
    );
    assert_eq!(paths(&both), ["copy/a.txt"]);
}

#[test]
fn size_and_time_filter_entries() {
    let mut s = setup();
    s.write("small.txt", b"12");
    s.write("big.txt", &[b'x'; 2000]);
    s.write("dir/mid.txt", &[b'y'; 100]);
    s.backup();

    let sized = |larger, smaller| {
        paths(&s.search(
            Query {
                larger,
                smaller,
                ..Query::default()
            },
            false,
        ))
    };
    assert_eq!(sized(Some(100), None), ["big.txt", "dir/mid.txt"]);
    assert_eq!(sized(None, Some(100)), ["small.txt"]);
    assert_eq!(sized(Some(100), Some(2000)), ["dir/mid.txt"]);

    let time = |after, before| {
        s.search(
            Query {
                after,
                before,
                ..Query::default()
            },
            false,
        )
        .hits
        .len()
    };
    let past = Time {
        seconds: 0,
        nanos: 0,
    };
    let future = Time {
        seconds: i64::from(u32::MAX),
        nanos: 0,
    };
    // Every file and the folder were modified after 1970.
    assert_eq!(time(Some(past), None), 4);
    assert_eq!(time(Some(future), None), 0);
    assert_eq!(time(None, Some(past)), 0);
}

#[test]
fn the_limit_stops_the_search_and_an_empty_query_is_refused() {
    let mut s = setup();
    for i in 0..5 {
        s.write(&format!("f{i}.txt"), b"x");
    }
    s.backup();
    let options = SearchOptions {
        starts: vec![s.repo.read_ref("main").unwrap().unwrap()],
        history: false,
        query: pattern("*"),
        limit: 3,
    };
    let out = search(&s.repo, &options, &Control::new()).unwrap();
    assert_eq!(out.hits.len(), 3);
    assert!(out.truncated);

    let empty = SearchOptions {
        query: Query::default(),
        ..options
    };
    assert!(matches!(
        search(&s.repo, &empty, &Control::new()),
        Err(Error::InvalidInput(_))
    ));
}
