//! Fault-injection tests for the interruption cases of FORMAT.md §10.
//!
//! Every change to the disk first calls [`checkpoint`]. A test arms a fault at
//! the n-th checkpoint of an operation and runs the operation once for every
//! n, on a fresh copy of the same repository each time.
//!
//! A simulated crash copies the repository as it is at that moment; the copy
//! is then opened like a repository after a power failure. The copy keeps data
//! that was written but not yet synced; torn writes are covered by their own
//! tests (`refs::tests::torn_reflog_line_is_ignored_and_truncated`).

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use crate::chunker::ChunkerParams;
use crate::chunker::tests::noise;
use crate::error::{Error, Result};
use crate::id::{ObjectId, ObjectType};
use crate::refs::RefOp;
use crate::repo::Repository;

enum Fault {
    /// Copies the repository at `repo` to `dest`, then fails.
    Crash { repo: PathBuf, dest: PathBuf },
    /// Fails like a full disk.
    DiskFull,
}

struct Plan {
    fault: Fault,
    remaining: usize,
    fired: bool,
}

thread_local! {
    static PLAN: RefCell<Option<Plan>> = const { RefCell::new(None) };
}

pub(crate) fn checkpoint() -> io::Result<()> {
    PLAN.with_borrow_mut(|plan| {
        let Some(plan) = plan.as_mut().filter(|p| !p.fired) else {
            return Ok(());
        };
        if plan.remaining > 0 {
            plan.remaining -= 1;
            return Ok(());
        }
        plan.fired = true;
        match &plan.fault {
            Fault::Crash { repo, dest } => {
                copy_dir(repo, dest);
                Err(io::Error::other("simulated crash"))
            }
            Fault::DiskFull => Err(io::ErrorKind::StorageFull.into()),
        }
    })
}

/// Copies `src` to `dest`; a `src` that does not exist yet copies as an
/// empty folder.
fn copy_dir(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    if !src.exists() {
        return;
    }
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Relative paths of all files below `dir`.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            out.extend(
                files(&entry.path())
                    .into_iter()
                    .map(|p| Path::new(&entry.file_name()).join(p)),
            );
        } else {
            out.push(PathBuf::from(entry.file_name()));
        }
    }
    out.sort();
    out
}

/// Makes every file below `dir` look two hours old, as after a long outage.
fn age_files(dir: &Path) {
    let old = SystemTime::now() - Duration::from_secs(2 * 3600);
    for path in files(dir) {
        fs::File::options()
            .write(true)
            .open(dir.join(path))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
}

const OLD: &[u8] = b"snapshot old";
const NEW: &[u8] = b"snapshot new";
const LATER: &[u8] = b"snapshot later";

/// A repository whose `main` ref points to the snapshot `old`.
struct Fixture {
    _dir: TempDir,
    base: PathBuf,
    old: ObjectId,
    new: ObjectId,
}

/// Password of the encrypted test repositories.
const PASSWORD: &str = "correct horse";

/// Creates a test repository, encrypted with [`PASSWORD`] if asked.
fn init(path: &Path, encrypted: bool) -> Repository {
    let params = ChunkerParams::new(64 * 1024).unwrap();
    if encrypted {
        Repository::init_encrypted(path, params, PASSWORD)
            .unwrap()
            .0
    } else {
        Repository::init_with(path, params).unwrap()
    }
}

/// Opens a test repository, encrypted or not.
fn open(path: &Path) -> Result<Repository> {
    Repository::unlock(path, PASSWORD)
}

fn fixture() -> Fixture {
    fixture_of(false)
}

fn fixture_of(encrypted: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("repo");
    let mut repo = init(&base, encrypted);
    let mut writer = repo.writer();
    let old = writer.put(ObjectType::Snapshot, OLD).unwrap();
    writer.finish().unwrap();
    repo.update_ref("main", None, Some(old)).unwrap();

    // Object ids depend on the repository's key: learn `new` from a copy.
    let scratch = tempfile::tempdir().unwrap();
    copy_dir(&base, scratch.path());
    let mut copy = open(scratch.path()).unwrap();
    let mut writer = copy.writer();
    let new = writer.put(ObjectType::Snapshot, NEW).unwrap();
    writer.finish().unwrap();
    Fixture {
        _dir: dir,
        base,
        old,
        new,
    }
}

/// The interrupted operation: store new data and move `main` to it.
fn commit_new(repo: &mut Repository, old: ObjectId) -> Result<()> {
    let mut writer = repo.writer();
    writer.put_chunks(noise(200 * 1024, 7).as_slice())?;
    let new = writer.put(ObjectType::Snapshot, NEW)?;
    writer.finish()?;
    repo.update_ref("main", Some(old), Some(new))
}

/// Runs [`commit_new`] on a fresh copy of the fixture with a fault at
/// checkpoint `n`. Returns the copy, the result and whether the fault fired.
fn run(f: &Fixture, n: usize, fault: impl FnOnce(&Path) -> Fault) -> (TempDir, Result<()>, bool) {
    run_on(&f.base, n, fault, |repo| commit_new(repo, f.old))
}

/// Runs `op` on a fresh copy of the repository at `base` with a fault at
/// checkpoint `n`. Returns the copy, the result and whether the fault fired.
fn run_on(
    base: &Path,
    n: usize,
    fault: impl FnOnce(&Path) -> Fault,
    op: impl FnOnce(&mut Repository) -> Result<()>,
) -> (TempDir, Result<()>, bool) {
    let work = tempfile::tempdir().unwrap();
    let path = work.path().join("repo");
    copy_dir(base, &path);
    let mut repo = open(&path).unwrap();
    PLAN.set(Some(Plan {
        fault: fault(&path),
        remaining: n,
        fired: false,
    }));
    let result = op(&mut repo);
    let fired = PLAN.take().is_some_and(|p| p.fired);
    (work, result, fired)
}

/// The repository as a crash at each checkpoint leaves it, in order,
/// followed by the state after the operation completed.
fn crash_states(f: &Fixture) -> Vec<TempDir> {
    crash_states_of(&f.base, |repo| commit_new(repo, f.old))
}

fn crash_states_of(base: &Path, op: impl Fn(&mut Repository) -> Result<()>) -> Vec<TempDir> {
    let mut states = Vec::new();
    for n in 0.. {
        let crash = tempfile::tempdir().unwrap();
        let dest = crash.path().join("repo");
        let (work, result, fired) = run_on(
            base,
            n,
            |repo| Fault::Crash {
                repo: repo.to_path_buf(),
                dest,
            },
            &op,
        );
        if !fired {
            result.unwrap();
            states.push(work);
            break;
        }
        assert!(result.is_err());
        states.push(crash);
    }
    states
}

fn repo_path(state: &TempDir) -> PathBuf {
    state.path().join("repo")
}

fn has_new_files(f: &Fixture, state: &Path, dir: &str) -> bool {
    files(&state.join(dir)) != files(&f.base.join(dir))
}

fn has_lock(state: &Path) -> bool {
    state.join("refs/main.lock").exists()
}

fn last_logged(repo: &Repository) -> Option<ObjectId> {
    repo.reflog("main").unwrap().last().and_then(|e| e.new)
}

/// Stores a further snapshot and moves `main` from `current` to it.
fn commit_later(repo: &mut Repository, current: ObjectId) -> ObjectId {
    let mut writer = repo.writer();
    let later = writer.put(ObjectType::Snapshot, LATER).unwrap();
    writer.finish().unwrap();
    repo.update_ref("main", Some(current), Some(later)).unwrap();
    later
}

/// Each reflog line continues from the previous one and the last matches the ref.
fn assert_reflog_chain(repo: &Repository) {
    let log = repo.reflog("main").unwrap();
    for pair in log.windows(2) {
        assert_eq!(pair[1].old, pair[0].new);
    }
    assert_eq!(
        log.last().and_then(|e| e.new),
        repo.read_ref("main").unwrap()
    );
}

#[test]
fn every_crash_leaves_main_at_a_readable_snapshot_and_recovers() {
    every_crash_leaves_main_at_a_readable_snapshot_and_recovers_with(false);
}

#[test]
fn every_crash_leaves_main_at_a_readable_snapshot_and_recovers_encrypted() {
    every_crash_leaves_main_at_a_readable_snapshot_and_recovers_with(true);
}

fn every_crash_leaves_main_at_a_readable_snapshot_and_recovers_with(encrypted: bool) {
    let f = fixture_of(encrypted);
    let states = crash_states(&f);
    assert!(states.len() > 20, "only {} checkpoints", states.len());

    let done = open(&repo_path(states.last().unwrap())).unwrap();
    assert_eq!(done.read_ref("main").unwrap(), Some(f.new));
    assert_reflog_chain(&done);

    for state in &states {
        let path = repo_path(state);
        let mut repo = open(&path).unwrap();
        let target = repo.read_ref("main").unwrap().unwrap();
        assert!(target == f.old || target == f.new);
        assert_eq!(repo.read_object(&target).unwrap().0, ObjectType::Snapshot);

        // After the outage: old temporary files and locks are cleaned up and
        // the repository accepts the next update.
        age_files(&path.join("tmp"));
        age_files(&path.join("refs"));
        repo.repair().unwrap();
        let mut repo = open(&path).unwrap();
        assert!(files(&path.join("tmp")).is_empty());
        commit_later(&mut repo, target);
        assert_reflog_chain(&repo);
    }
}

#[test]
fn crash_while_pack_is_in_tmp_changes_nothing() {
    let f = fixture();
    let mut seen = 0;
    for state in crash_states(&f) {
        let path = repo_path(&state);
        if has_new_files(&f, &path, "packs") {
            continue;
        }
        let repo = open(&path).unwrap();
        assert_eq!(repo.read_ref("main").unwrap(), Some(f.old));
        assert!(!repo.contains(&f.new));
        seen += usize::from(!files(&path.join("tmp")).is_empty());

        age_files(&path.join("tmp"));
        open(&path).unwrap();
        assert!(files(&path.join("tmp")).is_empty());
    }
    assert!(seen > 0);
}

#[test]
fn crash_after_pack_move_before_index_indexes_the_orphan_pack() {
    let f = fixture();
    let mut seen = 0;
    for state in crash_states(&f) {
        let path = repo_path(&state);
        if !has_new_files(&f, &path, "packs") || has_new_files(&f, &path, "index") {
            continue;
        }
        let repo = open(&path).unwrap();
        assert_eq!(repo.read_ref("main").unwrap(), Some(f.old));
        assert_eq!(repo.read_object(&f.new).unwrap().1, NEW);
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn crash_after_index_before_ref_update_keeps_old_ref() {
    let f = fixture();
    let mut seen = 0;
    for state in crash_states(&f) {
        let path = repo_path(&state);
        let repo = open(&path).unwrap();
        if !has_new_files(&f, &path, "index")
            || has_lock(&path)
            || repo.read_ref("main").unwrap() != Some(f.old)
        {
            continue;
        }
        assert_eq!(repo.read_object(&f.new).unwrap().1, NEW);
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn crash_with_ref_lock_taken_needs_repair() {
    let f = fixture();
    let mut seen = 0;
    for state in crash_states(&f) {
        let path = repo_path(&state);
        if !has_lock(&path) {
            continue;
        }
        let mut repo = open(&path).unwrap();
        assert_eq!(repo.read_ref("main").unwrap(), Some(f.old));
        assert!(matches!(
            repo.update_ref("main", Some(f.old), Some(f.new)),
            Err(Error::RefLocked { .. })
        ));

        // A fresh lock may belong to a running writer and is kept.
        assert!(repo.repair().unwrap().removed_locks.is_empty());
        assert!(has_lock(&path));

        age_files(&path.join("refs"));
        assert_eq!(
            repo.repair().unwrap().removed_locks,
            [path.join("refs/main.lock")]
        );
        commit_later(&mut repo, f.old);
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn crash_after_ref_move_before_reflog_gets_repair_line() {
    let f = fixture();
    let mut seen = 0;
    for state in crash_states(&f) {
        let path = repo_path(&state);
        let mut repo = open(&path).unwrap();
        if repo.read_ref("main").unwrap() != Some(f.new) || last_logged(&repo) != Some(f.old) {
            continue;
        }
        let later = commit_later(&mut repo, f.new);
        let log = repo.reflog("main").unwrap();
        let tail: Vec<_> = log[log.len() - 2..]
            .iter()
            .map(|e| (e.old, e.new, e.op))
            .collect();
        assert_eq!(
            tail,
            [
                (Some(f.old), Some(f.new), RefOp::Repair),
                (Some(f.new), Some(later), RefOp::Update),
            ]
        );
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn repair_command_appends_lost_reflog_line() {
    let f = fixture();
    let state = crash_states(&f)
        .into_iter()
        .find(|state| {
            let repo = open(&repo_path(state)).unwrap();
            repo.read_ref("main").unwrap() == Some(f.new) && last_logged(&repo) == Some(f.old)
        })
        .unwrap();
    let mut repo = open(&repo_path(&state)).unwrap();
    assert_eq!(repo.repair().unwrap().repaired_refs, ["main"]);
    let last = repo.reflog("main").unwrap().pop().unwrap();
    assert_eq!(
        (last.old, last.new, last.op),
        (Some(f.old), Some(f.new), RefOp::Repair)
    );
    assert_reflog_chain(&repo);
}

#[test]
fn disk_full_at_any_step_reports_error_and_cleans_up() {
    disk_full_at_any_step_reports_error_and_cleans_up_with(false);
}

#[test]
fn disk_full_at_any_step_reports_error_and_cleans_up_encrypted() {
    disk_full_at_any_step_reports_error_and_cleans_up_with(true);
}

fn disk_full_at_any_step_reports_error_and_cleans_up_with(encrypted: bool) {
    let f = fixture_of(encrypted);
    let mut steps = 0;
    for n in 0.. {
        let (work, result, fired) = run(&f, n, |_| Fault::DiskFull);
        if !fired {
            result.unwrap();
            break;
        }
        steps += 1;
        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::Io { source, .. } if source.kind() == io::ErrorKind::StorageFull),
            "{err}"
        );

        let path = repo_path(&work);
        assert!(files(&path.join("tmp")).is_empty(), "step {n}");
        assert!(!has_lock(&path), "step {n}");
        let mut repo = open(&path).unwrap();
        let target = repo.read_ref("main").unwrap().unwrap();
        // The ref only moves when the error hit the reflog append after it;
        // the line may or may not have been written.
        if target == f.new {
            assert!([Some(f.old), Some(f.new)].contains(&last_logged(&repo)));
        } else {
            assert_eq!(target, f.old);
        }
        assert_eq!(repo.read_object(&target).unwrap().0, ObjectType::Snapshot);
        commit_later(&mut repo, target);
        assert_reflog_chain(&repo);
    }
    assert!(steps > 20, "only {steps} checkpoints");
}

#[test]
fn bit_flip_anywhere_in_a_pack_is_never_silent() {
    bit_flip_anywhere_in_a_pack_is_never_silent_with(false);
}

#[test]
fn bit_flip_anywhere_in_a_pack_is_never_silent_encrypted() {
    bit_flip_anywhere_in_a_pack_is_never_silent_with(true);
}

fn bit_flip_anywhere_in_a_pack_is_never_silent_with(encrypted: bool) {
    let f = fixture_of(encrypted);
    let mut repo = open(&f.base).unwrap();
    let data = noise(4096, 8);
    let mut writer = repo.writer();
    let id = writer.put(ObjectType::Chunk, &data).unwrap();
    writer.finish().unwrap();

    let pack = files(&f.base.join("packs"))
        .into_iter()
        .map(|p| f.base.join("packs").join(p))
        .find(|p| fs::read(p).unwrap().len() > data.len())
        .unwrap();
    let index_files = files(&f.base.join("index"));
    let original = fs::read(&pack).unwrap();

    for i in 0..original.len() {
        let mut bytes = original.clone();
        bytes[i] ^= 0x01;
        fs::write(&pack, &bytes).unwrap();

        // Through the index the object is either intact or reported corrupt.
        match repo.read_object(&id) {
            Ok((_, read)) => assert_eq!(read, data, "byte {i}"),
            Err(Error::Corrupt { .. }) => {}
            Err(e) => panic!("byte {i}: {e}"),
        }

        // Without an index, the pack is checked against its name when opened:
        // it is reported as damaged and its object is not served.
        let hidden = tempfile::tempdir().unwrap();
        for name in &index_files {
            fs::rename(f.base.join("index").join(name), hidden.path().join(name)).unwrap();
        }
        let reopened = open(&f.base).unwrap();
        assert_eq!(
            reopened.damaged_files(),
            std::slice::from_ref(&pack),
            "byte {i}"
        );
        assert!(
            matches!(reopened.read_object(&id), Err(Error::ObjectNotFound(_))),
            "byte {i}"
        );
        for name in &index_files {
            fs::rename(hidden.path().join(name), f.base.join("index").join(name)).unwrap();
        }
    }
}

mod gc {
    use super::*;
    use crate::object::{EntryKind, Snapshot, Time, Tree, TreeEntry};
    use crate::repo::GcOptions;

    /// A repository where `keep` must survive garbage collection, `drop`
    /// was deleted, and one pack holds objects of both.
    struct GcFixture {
        _dir: TempDir,
        base: PathBuf,
        kept_data: Vec<u8>,
    }

    fn store(writer: &mut crate::repo::ObjectWriter<'_>, data: &[u8], message: &str) -> ObjectId {
        let (size, blob) = writer.put_file(data).unwrap();
        let tree = writer
            .put_tree(&Tree {
                entries: vec![TreeEntry {
                    name: b"file".to_vec(),
                    kind: EntryKind::File { size, blob },
                    mode: None,
                    mtime: None,
                    uid: None,
                    gid: None,
                    xattrs: vec![],
                }],
            })
            .unwrap();
        writer
            .put_snapshot(&Snapshot {
                tree,
                parents: vec![],
                time: Time {
                    seconds: 1_700_000_000,
                    nanos: 0,
                },
                author: None,
                host: None,
                message: Some(message.to_owned()),
                meta: vec![],
            })
            .unwrap()
    }

    fn gc_fixture() -> GcFixture {
        gc_fixture_of(false)
    }

    fn gc_fixture_of(encrypted: bool) -> GcFixture {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("repo");
        let mut repo = init(&base, encrypted);
        let kept_data = noise(150 * 1024, 11);

        let mut writer = repo.writer();
        let keep = store(&mut writer, &kept_data, "keep");
        let shared_drop = store(&mut writer, &noise(100 * 1024, 12), "drop");
        writer.finish().unwrap();
        let mut writer = repo.writer();
        let only_drop = store(&mut writer, &noise(100 * 1024, 13), "gone");
        writer.finish().unwrap();

        repo.update_ref("keep", None, Some(keep)).unwrap();
        repo.update_ref("drop", None, Some(shared_drop)).unwrap();
        repo.update_ref("gone", None, Some(only_drop)).unwrap();
        repo.update_ref("drop", Some(shared_drop), None).unwrap();
        repo.update_ref("gone", Some(only_drop), None).unwrap();
        GcFixture {
            _dir: dir,
            base,
            kept_data,
        }
    }

    fn collect(repo: &mut Repository) -> Result<()> {
        repo.gc(&GcOptions {
            keep_reflog_days: 0,
            dry_run: false,
        })
        .map(drop)
    }

    /// `keep` is intact, nothing is damaged, and another collection finishes.
    fn assert_healthy(path: &Path, kept_data: &[u8]) {
        let mut repo = open(path).unwrap();
        let check = |repo: &Repository| {
            let snapshot = repo
                .read_snapshot(&repo.read_ref("keep").unwrap().unwrap())
                .unwrap();
            let tree = repo.read_tree(&snapshot.tree).unwrap();
            let EntryKind::File {
                blob: Some(blob), ..
            } = tree.entries[0].kind
            else {
                panic!("expected a file");
            };
            let mut out = Vec::new();
            repo.read_file(&blob, &mut out).unwrap();
            assert_eq!(out, kept_data);
            assert!(repo.verify().unwrap().is_ok());
        };
        check(&repo);
        collect(&mut repo).unwrap();
        check(&repo);
        let again = repo
            .gc(&GcOptions {
                keep_reflog_days: 0,
                dry_run: true,
            })
            .unwrap();
        assert_eq!(again.removed_objects, 0);
    }

    #[test]
    fn every_crash_during_gc_keeps_live_data() {
        every_crash_during_gc_keeps_live_data_with(false);
    }

    #[test]
    fn every_crash_during_gc_keeps_live_data_encrypted() {
        every_crash_during_gc_keeps_live_data_with(true);
    }

    fn every_crash_during_gc_keeps_live_data_with(encrypted: bool) {
        let f = gc_fixture_of(encrypted);
        let states = crash_states_of(&f.base, collect);
        assert!(states.len() > 10, "only {} checkpoints", states.len());
        for state in &states {
            assert_healthy(&repo_path(state), &f.kept_data);
        }
    }

    #[test]
    fn disk_full_during_gc_keeps_live_data() {
        let f = gc_fixture();
        for n in 0.. {
            let (work, result, fired) = run_on(&f.base, n, |_| Fault::DiskFull, collect);
            if !fired {
                result.unwrap();
                break;
            }
            assert!(result.is_err());
            assert_healthy(&repo_path(&work), &f.kept_data);
        }
    }

    fn copy_of(f: &GcFixture) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        copy_dir(&f.base, &path);
        (dir, path)
    }

    #[test]
    fn gc_removes_dead_objects_and_rewrites_the_shared_pack() {
        let f = gc_fixture();
        let (_dir, path) = copy_of(&f);
        let mut repo = open(&path).unwrap();
        // The dry run creates only the lock file, which is never deleted.
        crate::lock::RepoLock::shared(&path).unwrap();
        let before = files(&path);
        let dry = repo
            .gc(&GcOptions {
                keep_reflog_days: 0,
                dry_run: true,
            })
            .unwrap();
        assert_eq!(files(&path), before);
        assert_eq!(dry.removed_packs, 2);
        assert!(dry.removed_objects > 0);

        let done = repo
            .gc(&GcOptions {
                keep_reflog_days: 0,
                dry_run: false,
            })
            .unwrap();
        assert_eq!((done.removed_packs, done.written_packs), (2, 1));
        assert_eq!(done.removed_objects, dry.removed_objects);
        assert_eq!(files(&path.join("packs")).len(), 1);
        assert_eq!(files(&path.join("index")).len(), 1);
        assert_healthy(&path, &f.kept_data);
    }

    #[test]
    fn recent_reflog_lines_keep_deleted_snapshots() {
        let f = gc_fixture();
        let (_dir, path) = copy_of(&f);
        let mut repo = open(&path).unwrap();
        let report = repo.gc(&GcOptions::default()).unwrap();
        assert_eq!(report.removed_objects, 0);
    }

    #[test]
    fn gc_needs_the_lock_alone() {
        let f = gc_fixture();
        let (_dir, path) = copy_of(&f);
        let mut repo = open(&path).unwrap();
        let reader = crate::lock::RepoLock::shared(&path).unwrap();
        assert!(matches!(
            repo.gc(&GcOptions::default()),
            Err(Error::RepoLocked(_))
        ));
        drop(reader);
        repo.gc(&GcOptions::default()).unwrap();
    }

    #[test]
    fn missing_reachable_object_stops_gc_before_deleting() {
        let f = gc_fixture();
        let (_dir, path) = copy_of(&f);
        let mut repo = open(&path).unwrap();
        // A tree naming a blob that does not exist, stored without the usual checks.
        let tree = Tree {
            entries: vec![TreeEntry {
                name: b"lost".to_vec(),
                kind: EntryKind::File {
                    size: 1,
                    blob: Some(ObjectId::from_bytes([7; 32])),
                },
                mode: None,
                mtime: None,
                uid: None,
                gid: None,
                xattrs: vec![],
            }],
        };
        let mut writer = repo.writer();
        let tree = writer
            .put(ObjectType::Tree, &tree.encode().unwrap())
            .unwrap();
        let snapshot = Snapshot {
            tree,
            parents: vec![],
            time: Time {
                seconds: 0,
                nanos: 0,
            },
            author: None,
            host: None,
            message: None,
            meta: vec![],
        };
        let snapshot = writer.put_snapshot(&snapshot).unwrap();
        writer.finish().unwrap();
        repo.update_ref("broken", None, Some(snapshot)).unwrap();

        let before = files(&path.join("packs"));
        let result = repo.gc(&GcOptions {
            keep_reflog_days: 0,
            dry_run: false,
        });
        assert!(matches!(result, Err(Error::ObjectNotFound(_))));
        assert_eq!(files(&path.join("packs")), before);
    }
}

#[test]
fn damaged_index_is_skipped_and_rebuilt_by_repair() {
    damaged_index_is_skipped_and_rebuilt_by_repair_with(false);
}

#[test]
fn damaged_index_is_skipped_and_rebuilt_by_repair_encrypted() {
    damaged_index_is_skipped_and_rebuilt_by_repair_with(true);
}

fn damaged_index_is_skipped_and_rebuilt_by_repair_with(encrypted: bool) {
    let f = fixture_of(encrypted);
    let index_dir = f.base.join("index");
    let index = index_dir.join(&files(&index_dir)[0]);
    let mut bytes = fs::read(&index).unwrap();
    bytes[40] ^= 0x01;
    fs::write(&index, bytes).unwrap();

    let mut repo = open(&f.base).unwrap();
    assert_eq!(repo.damaged_files(), std::slice::from_ref(&index));
    assert_eq!(repo.read_object(&f.old).unwrap().1, OLD);
    assert_eq!(
        repo.verify().unwrap().damaged_files,
        std::slice::from_ref(&index)
    );

    let report = repo.repair().unwrap();
    assert_eq!(report.removed_index_files, [index]);
    assert_eq!(report.indexed_packs, 1);
    let repo = open(&f.base).unwrap();
    assert!(repo.damaged_files().is_empty());
    // The fixture's snapshots are plain bytes, so only files are checked here.
    assert!(repo.verify().unwrap().damaged_files.is_empty());
    assert_eq!(repo.read_object(&f.old).unwrap().1, OLD);
}

#[test]
fn every_crash_during_index_repair_keeps_objects_readable() {
    every_crash_during_index_repair_keeps_objects_readable_with(false);
}

#[test]
fn every_crash_during_index_repair_keeps_objects_readable_encrypted() {
    every_crash_during_index_repair_keeps_objects_readable_with(true);
}

fn every_crash_during_index_repair_keeps_objects_readable_with(encrypted: bool) {
    let f = fixture_of(encrypted);
    let index_dir = f.base.join("index");
    let index = index_dir.join(&files(&index_dir)[0]);
    let mut bytes = fs::read(&index).unwrap();
    bytes[40] ^= 0x01;
    fs::write(&index, bytes).unwrap();

    let states = crash_states_of(&f.base, |repo| repo.repair().map(drop));
    assert!(states.len() > 3);
    for state in &states {
        let mut repo = open(&repo_path(state)).unwrap();
        assert_eq!(repo.read_object(&f.old).unwrap().1, OLD);
        repo.repair().unwrap();
        let repo = open(&repo_path(state)).unwrap();
        assert!(repo.damaged_files().is_empty());
        assert!(repo.verify().unwrap().damaged_files.is_empty());
    }
}

#[test]
fn every_crash_during_encrypted_init_leaves_no_repository_or_a_whole_one() {
    let params = ChunkerParams::new(64 * 1024).unwrap();
    let mut whole = 0;
    for n in 0.. {
        let work = tempfile::tempdir().unwrap();
        let path = work.path().join("repo");
        let crash = tempfile::tempdir().unwrap();
        let dest = crash.path().join("repo");
        PLAN.set(Some(Plan {
            fault: Fault::Crash {
                repo: path.clone(),
                dest: dest.clone(),
            },
            remaining: n,
            fired: false,
        }));
        let result = Repository::init_encrypted(&path, params, PASSWORD);
        if !PLAN.take().is_some_and(|p| p.fired) {
            result.unwrap();
            break;
        }
        assert!(result.is_err());
        match open(&dest) {
            Ok(repo) => {
                assert!(repo.is_encrypted());
                whole += 1;
            }
            Err(Error::InvalidRepository { reason, .. }) => assert_eq!(reason, "no config file"),
            Err(e) => panic!("checkpoint {n}: {e}"),
        }
    }
    assert!(whole > 0);
}
