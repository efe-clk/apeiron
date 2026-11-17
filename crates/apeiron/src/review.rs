//! Reviews: asking for a branch to be merged, comments, approvals and the
//! merge (docs/REVIEW.md). Each step is a file of its own in the ref
//! `apeiron/reviews`, so steps written at the same time never replace each
//! other, and copies of the repository carry them.

use std::collections::BTreeMap;
use std::hash::{BuildHasher, RandomState};

use apeiron_core as core;
use apeiron_services as services;
use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind, Result};
use crate::{ListRequest, MergeReport, MergeRequest, Name, ObjectId, Repository};

/// The ref reviews are kept in.
pub const REVIEWS_REF: &str = "apeiron/reviews";

const MAX_TITLE: usize = 200;
const MAX_TEXT: usize = 10_000;

/// What a step of a review does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
    Open,
    Comment,
    Approve,
    RequestChanges,
    Merge,
    Close,
}

/// One step of a review, as it is stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReviewEvent {
    pub action: ReviewAction,
    pub author: String,
    /// Unix time in seconds.
    pub time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The file a comment is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// For `open`, `approve`, `request_changes` and `merge`: the snapshot
    /// `from` pointed to then; a merge merges that one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<ObjectId>,
    /// For `open` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Open,
    Merged,
    Closed,
}

/// A review as its steps make it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Review {
    pub id: String,
    pub title: String,
    /// The ref whose changes are asked to be merged.
    pub from: String,
    /// The ref they would be merged into.
    pub into: String,
    pub author: String,
    /// Unix time in seconds when it was opened.
    pub opened: u64,
    pub state: ReviewState,
    /// What `from` points to now; for a merged review, what was merged.
    /// `None` when the ref is gone.
    pub snapshot: Option<ObjectId>,
    /// Who approved `snapshot`; an approval of an older snapshot does not
    /// count.
    pub approvals: Vec<String>,
    /// Who asked for changes and has not approved since.
    pub changes_requested: Vec<String>,
    /// Whether `merge_review` would merge it now.
    pub ready: bool,
    /// Every step, oldest first.
    pub events: Vec<ReviewEvent>,
}

/// Asks for the changes of a ref to be merged into another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OpenReviewRequest {
    pub from: String,
    #[serde(default = "main")]
    pub into: String,
    pub title: String,
    #[serde(default)]
    pub text: Option<String>,
    /// The operating system user when absent.
    #[serde(default)]
    pub author: Option<String>,
}

fn main() -> String {
    "main".to_owned()
}

/// A comment, an approval, a request for changes or closing a review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReviewNoteRequest {
    /// The id of the review or its first characters.
    pub id: String,
    /// `comment`, `approve`, `request_changes` or `close`.
    pub action: ReviewAction,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

/// Which reviews to list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReviewsRequest {
    /// Merged and closed ones too.
    #[serde(default)]
    pub all: bool,
}

/// Merges an approved review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MergeReviewRequest {
    pub id: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

/// The review after the merge, and the merge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReviewMerged {
    pub review: Review,
    pub merge: MergeReport,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn conflict(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Conflict, message)
}

/// `author`, or the operating system user.
fn author_or_user(author: Option<&str>) -> Result<String> {
    let name = match author {
        Some(a) => a.trim().to_owned(),
        None => ["USER", "USERNAME"]
            .iter()
            .find_map(|v| std::env::var(v).ok())
            .unwrap_or_default(),
    };
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(invalid("give an author: one line of text"));
    }
    Ok(name)
}

fn check_text(text: Option<&str>) -> Result<Option<String>> {
    match text.map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) if t.chars().count() > MAX_TEXT => Err(invalid(format!(
            "the text is longer than {MAX_TEXT} characters"
        ))),
        Some(t) => Ok(Some(t.to_owned())),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// 16 hex characters no one can guess ahead: a hash with a key the
/// standard library picks at random.
fn random_hex(seed: &str) -> String {
    let n = RandomState::new().hash_one((seed, std::process::id(), now()));
    format!("{n:016x}")
}

/// The review id of a step's file, named `<id>-<nanoseconds>-<random>.json`.
fn parse_name(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".json")?;
    let (id, _) = stem.split_once('-')?;
    (id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// Builds a review from its steps.
fn build(id: &str, events: Vec<ReviewEvent>, current: Option<ObjectId>) -> Option<Review> {
    let open = events.first().filter(|e| e.action == ReviewAction::Open)?;
    let mut review = Review {
        id: id.to_owned(),
        title: open.title.clone().unwrap_or_default(),
        from: open.from.clone().unwrap_or_default(),
        into: open.into.clone().unwrap_or_default(),
        author: open.author.clone(),
        opened: open.time,
        state: ReviewState::Open,
        snapshot: current,
        approvals: Vec::new(),
        changes_requested: Vec::new(),
        ready: false,
        events: Vec::new(),
    };
    // The latest verdict of each person, with the snapshot it was given on.
    let mut verdicts: BTreeMap<String, (ReviewAction, Option<ObjectId>)> = BTreeMap::new();
    let mut merged = None;
    for e in &events {
        match e.action {
            ReviewAction::Approve | ReviewAction::RequestChanges => {
                verdicts.insert(e.author.clone(), (e.action, e.snapshot));
            }
            ReviewAction::Merge if review.state == ReviewState::Open => {
                review.state = ReviewState::Merged;
                merged = e.snapshot;
            }
            ReviewAction::Close if review.state == ReviewState::Open => {
                review.state = ReviewState::Closed;
            }
            _ => {}
        }
    }
    if review.state == ReviewState::Merged {
        review.snapshot = merged;
    }
    for (who, (action, snapshot)) in verdicts {
        match action {
            ReviewAction::Approve if snapshot.is_some() && snapshot == review.snapshot => {
                review.approvals.push(who);
            }
            ReviewAction::RequestChanges => review.changes_requested.push(who),
            _ => {}
        }
    }
    review.ready = review.state == ReviewState::Open
        && review.snapshot.is_some()
        && !review.approvals.is_empty()
        && review.changes_requested.is_empty();
    review.events = events;
    Some(review)
}

impl Repository {
    /// Every review, newest first; only open ones unless `all`.
    pub fn reviews(&self, request: &ReviewsRequest) -> Result<Vec<Review>> {
        let mut all = self.load_reviews()?;
        all.retain(|r| request.all || r.state == ReviewState::Open);
        all.sort_by(|a, b| b.opened.cmp(&a.opened).then(a.id.cmp(&b.id)));
        Ok(all)
    }

    /// One review, by its id or the first characters of it.
    pub fn review(&self, id: &str) -> Result<Review> {
        let matches: Vec<Review> = self
            .load_reviews()?
            .into_iter()
            .filter(|r| !id.is_empty() && r.id.starts_with(id))
            .collect();
        match <[Review; 1]>::try_from(matches) {
            Ok([r]) => Ok(r),
            Err(m) if m.is_empty() => Err(Error::new(
                ErrorKind::NotFound,
                format!("no review with id {id:?}"),
            )),
            Err(_) => Err(Error::new(
                ErrorKind::Ambiguous,
                format!("{id:?} is the start of more than one review id"),
            )),
        }
    }

    /// Asks for the changes of `from` to be merged into `into`.
    pub fn open_review(&self, request: &OpenReviewRequest) -> Result<Review> {
        let detail = Some(format!("{} {}", request.from, request.into));
        let result = (|| {
            let title = request.title.trim();
            if title.is_empty() || title.chars().count() > MAX_TITLE || title.contains('\n') {
                return Err(invalid(format!(
                    "the title must be one line of 1 to {MAX_TITLE} characters"
                )));
            }
            let author = author_or_user(request.author.as_deref())?;
            for name in [&request.from, &request.into] {
                core::validate_ref_name(name)?;
            }
            if request.from == request.into {
                return Err(invalid("a ref cannot be merged into itself"));
            }
            let snapshot = self.resolve(&request.from)?;
            self.resolve(&request.into)?;
            let taken: Vec<String> = self.load_reviews()?.into_iter().map(|r| r.id).collect();
            let id = loop {
                let id = random_hex(&request.title)[..8].to_owned();
                if !taken.contains(&id) {
                    break id;
                }
            };
            self.put_event(
                &id,
                &ReviewEvent {
                    action: ReviewAction::Open,
                    author,
                    time: now(),
                    text: check_text(request.text.as_deref())?,
                    path: None,
                    snapshot: Some(snapshot),
                    from: Some(request.from.clone()),
                    into: Some(request.into.clone()),
                    title: Some(title.to_owned()),
                },
            )?;
            self.review(&id)
        })();
        self.recorded("open-review", None, detail, result)
    }

    /// Adds a comment, an approval or a request for changes to a review,
    /// or closes it. Only comments are taken once it is merged or closed;
    /// its author cannot approve it.
    pub fn note_review(&self, request: &ReviewNoteRequest) -> Result<Review> {
        let detail = Some(request.id.clone());
        let result = (|| {
            let review = self.review(&request.id)?;
            let author = author_or_user(request.author.as_deref())?;
            let text = check_text(request.text.as_deref())?;
            let mut snapshot = None;
            match request.action {
                ReviewAction::Comment => {
                    if text.is_none() {
                        return Err(invalid("a comment needs text"));
                    }
                }
                ReviewAction::Approve | ReviewAction::RequestChanges | ReviewAction::Close => {
                    if review.state != ReviewState::Open {
                        return Err(conflict(format!("review {} is not open", review.id)));
                    }
                    if request.action == ReviewAction::Approve && author == review.author {
                        return Err(invalid("the author of a review cannot approve it"));
                    }
                    if request.action != ReviewAction::Close {
                        snapshot = Some(review.snapshot.ok_or_else(|| {
                            conflict(format!("the ref {} no longer exists", review.from))
                        })?);
                    }
                }
                ReviewAction::Open | ReviewAction::Merge => {
                    return Err(invalid("use open_review or merge_review for that step"));
                }
            }
            let path = match request.path.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(_) if request.action != ReviewAction::Comment => {
                    return Err(invalid("only a comment can name a file"));
                }
                Some(p) if p.contains('\n') => return Err(invalid("a path is one line")),
                Some(p) => Some(p.to_owned()),
            };
            self.put_event(
                &review.id,
                &ReviewEvent {
                    action: request.action,
                    author,
                    time: now(),
                    text,
                    path,
                    snapshot,
                    from: None,
                    into: None,
                    title: None,
                },
            )?;
            self.review(&review.id)
        })();
        self.recorded("note-review", None, detail, result)
    }

    /// Merges a review that is ready: approved on the snapshot its ref
    /// points to now, with no request for changes left. The merge must not
    /// make conflict copies; then fails with [`ErrorKind::Conflict`] and
    /// changes nothing.
    pub fn merge_review(&self, request: &MergeReviewRequest) -> Result<ReviewMerged> {
        let detail = Some(request.id.clone());
        let result = (|| {
            let review = self.review(&request.id)?;
            let author = author_or_user(request.author.as_deref())?;
            let snapshot = match (review.state, review.snapshot) {
                (ReviewState::Open, Some(s)) if review.ready => s,
                (ReviewState::Open, _) => {
                    return Err(conflict(format!(
                        "review {} is not ready: it needs an approval of what {} holds now and no request for changes",
                        review.id, review.from
                    )));
                }
                _ => return Err(conflict(format!("review {} is not open", review.id))),
            };
            let merge = MergeRequest {
                from: snapshot.to_string(),
                into: review.into.clone(),
                message: Some(request.message.clone().unwrap_or_else(|| {
                    format!(
                        "merge {} into {}: {}",
                        review.from, review.into, review.title
                    )
                })),
                author: Some(author.clone()),
                dry_run: true,
            };
            let trial = self.merge(&merge)?;
            if !trial.conflicts.is_empty() {
                return Err(conflict(format!(
                    "merging {} into {} makes {} conflict copies; merge {} into {} first and ask for an approval again",
                    review.from,
                    review.into,
                    trial.conflicts.len(),
                    review.into,
                    review.from
                )));
            }
            let report = self.merge(&MergeRequest {
                dry_run: false,
                ..merge
            })?;
            self.put_event(
                &review.id,
                &ReviewEvent {
                    action: ReviewAction::Merge,
                    author,
                    time: now(),
                    text: None,
                    path: None,
                    snapshot: Some(snapshot),
                    from: None,
                    into: None,
                    title: None,
                },
            )?;
            Ok(ReviewMerged {
                review: self.review(&review.id)?,
                merge: report,
            })
        })();
        self.recorded("merge-review", None, detail, result)
    }

    fn put_event(&self, id: &str, event: &ReviewEvent) -> Result<()> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!("{id}-{nanos:020}-{}.json", random_hex(id));
        let data = serde_json::to_vec_pretty(event)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let _writer = self.lock_writer();
        let _lock = core::RepoLock::shared(self.path())?;
        let mut core = self.read().reopen()?;
        let message = format!("review {id}: {:?}", event.action).to_lowercase();
        let stored = services::put_files(
            &mut core,
            REVIEWS_REF,
            &[(name.into_bytes(), data)],
            &[],
            &message,
        );
        *self.write() = core;
        stored?;
        Ok(())
    }

    fn load_reviews(&self) -> Result<Vec<Review>> {
        let entries = match self.list(&ListRequest {
            snapshot: REVIEWS_REF.to_owned(),
            path: Name::default(),
        }) {
            Ok(entries) => entries,
            Err(e) if e.kind == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut steps: BTreeMap<String, Vec<(String, ReviewEvent)>> = BTreeMap::new();
        for e in entries {
            let name = e.name.to_string();
            let Some(id) = parse_name(&name) else {
                continue;
            };
            let mut data = Vec::new();
            self.read_file(
                &ListRequest {
                    snapshot: REVIEWS_REF.to_owned(),
                    path: e.name.clone(),
                },
                &mut data,
            )?;
            let event: ReviewEvent = serde_json::from_slice(&data).map_err(|err| {
                Error::new(
                    ErrorKind::Corrupt,
                    format!("{REVIEWS_REF}/{name} is not a review step: {err}"),
                )
            })?;
            steps
                .entry(id.to_owned())
                .or_default()
                .push((name.clone(), event));
        }
        let mut reviews = Vec::new();
        for (id, mut events) in steps {
            // Opening first, then by name, which starts with the time in
            // nanoseconds, so every copy shows the same order.
            events.sort_by(|a, b| {
                let opens = |e: &ReviewEvent| e.action != ReviewAction::Open;
                opens(&a.1).cmp(&opens(&b.1)).then(a.0.cmp(&b.0))
            });
            let events: Vec<ReviewEvent> = events.into_iter().map(|(_, e)| e).collect();
            let from = events.first().and_then(|e| e.from.clone());
            let current = match from {
                Some(f) => match self.resolve(&f) {
                    Ok(id) => Some(id),
                    Err(e) if e.kind == ErrorKind::NotFound => None,
                    Err(e) => return Err(e),
                },
                None => None,
            };
            if let Some(review) = build(&id, events, current) {
                reviews.push(review);
            }
        }
        Ok(reviews)
    }
}
