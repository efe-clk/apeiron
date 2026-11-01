//! `git clone`, `git fetch` and `git push` over HTTP from the web server
//! (GIT.md §7): Git's smart protocol, version 0.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use apeiron::{Error, ErrorKind, GitPushedRef, GitRequest, GitServedRef, Repository};
use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::audit::Actor;
use crate::users::{Role, User};
use crate::web::WebState;
use crate::{App, Failure, Mode, blocking, lock, peer};

/// One export into a Git copy at a time.
static EXPORTING: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
pub(crate) struct Service {
    service: Option<String>,
}

/// `GET /git/<prefix>/info/refs?service=git-upload-pack` or
/// `git-receive-pack`: the branches and tags under `<prefix>` the user
/// sees, to fetch or to push to.
pub(crate) async fn info_refs(
    State(app): State<Arc<App>>,
    UrlPath(prefix): UrlPath<String>,
    Query(query): Query<Service>,
    request: Request,
) -> Response {
    let push = match query.service.as_deref() {
        Some("git-upload-pack") => false,
        Some("git-receive-pack") => true,
        _ => {
            return text(
                StatusCode::FORBIDDEN,
                "only git clone, git fetch and git push are served; upgrade Git if it asks otherwise",
            );
        }
    };
    let from = peer(&request);
    let (user, repo, cache, prefix) = match signed_in(&app, &prefix, request.headers(), from).await
    {
        Ok(found) => found,
        Err(response) => return *response,
    };
    if push && let Err(response) = may_push(&app, &user) {
        return *response;
    }
    let refs = match visible_refs(&repo, &user, &cache, &prefix).await {
        Ok(refs) => refs,
        Err(e) => return Failure::from(e).into_response(),
    };
    let service = if push {
        "git-receive-pack"
    } else {
        "git-upload-pack"
    };
    let mut out = Vec::new();
    pkt(&mut out, format!("# service={service}\n").as_bytes());
    out.extend_from_slice(b"0000");
    let agent = format!("agent=apeiron/{}", env!("CARGO_PKG_VERSION"));
    // A push gets a report per ref, and no packs that lean on objects
    // outside them.
    let mut caps = if push {
        format!("report-status no-thin {agent}")
    } else {
        agent
    };
    let head = refs
        .iter()
        .find(|r| r.git == "refs/heads/main")
        .or_else(|| refs.iter().find(|r| r.git == "refs/heads/master"))
        .or_else(|| refs.iter().find(|r| r.git.starts_with("refs/heads/")))
        .filter(|_| !push);
    match head {
        Some(head) => {
            caps = format!("symref=HEAD:{} {caps}", head.git);
            pkt(&mut out, format!("{} HEAD\0{caps}\n", head.id).as_bytes());
            for r in &refs {
                pkt(&mut out, format!("{} {}\n", r.id, r.git).as_bytes());
            }
        }
        None => match refs.split_first() {
            Some((first, rest)) => {
                pkt(
                    &mut out,
                    format!("{} {}\0{caps}\n", first.id, first.git).as_bytes(),
                );
                for r in rest {
                    pkt(&mut out, format!("{} {}\n", r.id, r.git).as_bytes());
                }
            }
            None => pkt(
                &mut out,
                format!("{} capabilities^{{}}\0{caps}\n", "0".repeat(40)).as_bytes(),
            ),
        },
    }
    out.extend_from_slice(b"0000");
    let content_type = if push {
        "application/x-git-receive-pack-advertisement"
    } else {
        "application/x-git-upload-pack-advertisement"
    };
    git_response(content_type, Body::from(out))
}

/// Whether the user may push: a writing role, on a server that takes
/// backups (USERS.md §1, REPLICATION.md §5).
#[allow(clippy::result_large_err)]
fn may_push(app: &App, user: &User) -> Result<(), Box<Response>> {
    if user.role == Role::Reader {
        return Err(Box::new(text(
            StatusCode::FORBIDDEN,
            "a reader cannot push",
        )));
    }
    if let Mode::Web(web) = &app.mode
        && web.standby()
    {
        return Err(Box::new(text(
            StatusCode::FORBIDDEN,
            "this is a standby copy; push to the primary",
        )));
    }
    Ok(())
}

/// `POST /git/<prefix>/git-receive-pack`: the refs to change and a pack of
/// their objects; each ref the user owns moves forward or is refused.
pub(crate) async fn receive_pack(
    State(app): State<Arc<App>>,
    UrlPath(prefix): UrlPath<String>,
    request: Request,
) -> Response {
    let from = peer(&request);
    let (user, repo, cache, prefix) =
        match signed_in(&app, &prefix, request.headers(), from.clone()).await {
            Ok(found) => found,
            Err(response) => return *response,
        };
    if let Err(response) = may_push(&app, &user) {
        return *response;
    }
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), PUSH_LIMIT).await else {
        return text(
            StatusCode::PAYLOAD_TOO_LARGE,
            "the push is larger than 256 MiB; push fewer commits at a time",
        );
    };
    let Some((updates, pack)) = parse_commands(&bytes) else {
        return text(StatusCode::BAD_REQUEST, "not a Git receive-pack request");
    };
    let results = {
        let (repo, prefix, cache) = (repo.clone(), prefix.clone(), cache.clone());
        let user = user.clone();
        let updates = updates.clone();
        let pack = pack.to_vec();
        blocking(move || {
            let _one = lock(&EXPORTING);
            let request = GitRequest {
                path: cache,
                prefix,
            };
            repo.git_receive(&request, &updates, &pack, &|name| user.owns(name))
        })
        .await
    };
    let mut out = Vec::new();
    let results = match results {
        Ok(results) => {
            pkt(&mut out, b"unpack ok\n");
            for r in &results {
                let line = match &r.error {
                    None => format!("ok {}\n", r.git),
                    Some(e) => format!("ng {} {}\n", r.git, e.replace('\n', " ")),
                };
                pkt(&mut out, line.as_bytes());
            }
            results
        }
        Err(e) => {
            pkt(
                &mut out,
                format!("unpack {}\n", e.message.replace('\n', " ")).as_bytes(),
            );
            for u in &updates {
                pkt(
                    &mut out,
                    format!("ng {} unpacker error\n", u.git).as_bytes(),
                );
            }
            Vec::new()
        }
    };
    out.extend_from_slice(b"0000");
    if let Mode::Web(web) = &app.mode {
        let actor = Actor {
            user: Some(user.name.clone()),
            from,
        };
        let refs: Vec<_> = updates
            .iter()
            .map(|u| {
                let error = results
                    .iter()
                    .find(|r| r.git == u.git)
                    .map_or(Some("unpacker error"), |r| r.error.as_deref());
                json!({"git": u.git, "new": u.new, "error": error})
            })
            .collect();
        let moved = results.iter().any(|r| r.error.is_none());
        let outcome = if moved { "ok" } else { "conflict" };
        if let Err(e) = web.audit.record(
            &actor,
            "git_push",
            json!({ "prefix": prefix, "refs": refs }),
            outcome,
        ) {
            return Failure::from(e).into_response();
        }
        if moved {
            web.replicate_now();
        }
    }
    git_response("application/x-git-receive-pack-result", Body::from(out))
}

/// The largest push taken in one request.
const PUSH_LIMIT: usize = 256 << 20;

/// The ref changes of a push and the pack after them.
fn parse_commands(mut data: &[u8]) -> Option<(Vec<GitPushedRef>, &[u8])> {
    let mut updates = Vec::new();
    loop {
        let len = usize::from_str_radix(std::str::from_utf8(data.get(..4)?).ok()?, 16).ok()?;
        if len == 0 {
            data = &data[4..];
            break;
        }
        let line = data.get(4..len)?;
        data = &data[len..];
        // The first line carries the client's capabilities after a NUL.
        let line = line.split(|&b| b == 0).next()?;
        let line = std::str::from_utf8(line).ok()?.trim_end_matches('\n');
        let mut parts = line.splitn(3, ' ');
        let (old, new, git) = (parts.next()?, parts.next()?, parts.next()?);
        let hex = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !hex(old) || !hex(new) {
            return None;
        }
        updates.push(GitPushedRef {
            git: git.to_owned(),
            old: old.to_ascii_lowercase(),
            new: new.to_ascii_lowercase(),
        });
    }
    (!updates.is_empty()).then_some((updates, data))
}

/// `POST /git/<prefix>/git-upload-pack`: a pack of what the client wants,
/// once it says `done`.
pub(crate) async fn upload_pack(
    State(app): State<Arc<App>>,
    UrlPath(prefix): UrlPath<String>,
    request: Request,
) -> Response {
    let from = peer(&request);
    let (user, repo, cache, prefix) =
        match signed_in(&app, &prefix, request.headers(), from.clone()).await {
            Ok(found) => found,
            Err(response) => return *response,
        };
    let headers = request.headers().clone();
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 64 << 20).await else {
        return text(StatusCode::BAD_REQUEST, "the request could not be read");
    };
    let gzip = headers
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v.as_bytes() == b"gzip");
    let body = if gzip {
        let mut plain = Vec::new();
        let limited = flate2::read::GzDecoder::new(&bytes[..]).take(64 << 20);
        if std::io::BufReader::new(limited)
            .read_to_end(&mut plain)
            .is_err()
        {
            return text(StatusCode::BAD_REQUEST, "the request is not valid gzip");
        }
        plain
    } else {
        bytes.to_vec()
    };
    let Some(request) = parse_request(&body) else {
        return text(StatusCode::BAD_REQUEST, "not a Git upload-pack request");
    };
    let refs = match visible_refs(&repo, &user, &cache, &prefix).await {
        Ok(refs) => refs,
        Err(e) => return Failure::from(e).into_response(),
    };
    // Only the tips offered may be asked for, as Git does by default; what
    // they reach is visible to the user (USERS.md §1).
    if let Some(other) = request
        .wants
        .iter()
        .find(|w| !refs.iter().any(|r| &r.id == *w))
    {
        return text(
            StatusCode::FORBIDDEN,
            &format!("{other} is not the tip of a branch or tag you can see"),
        );
    }
    // The first commit both sides have ends the talk, as in Git without
    // `multi_ack`; the client then says `done`, and the pack leaves out
    // what the common commits reach.
    let common = {
        let (repo, cache) = (repo.clone(), cache.clone());
        let (wants, haves) = (request.wants.clone(), request.haves.clone());
        match blocking(move || repo.git_common(&cache, &wants, &haves)).await {
            Ok(common) => common,
            Err(e) => return Failure::from(e).into_response(),
        }
    };
    let mut answer = Vec::new();
    match common.first() {
        Some(id) => pkt(&mut answer, format!("ACK {id}\n").as_bytes()),
        None => pkt(&mut answer, b"NAK\n"),
    }
    if !request.done {
        return git_response("application/x-git-upload-pack-result", Body::from(answer));
    }
    if let Mode::Web(web) = &app.mode {
        let actor = Actor {
            user: Some(user.name.clone()),
            from,
        };
        let names: Vec<&str> = refs
            .iter()
            .filter(|r| request.wants.contains(&r.id))
            .map(|r| r.ref_name.as_str())
            .collect();
        if let Err(e) = web.audit.record(
            &actor,
            "git_fetch",
            json!({ "prefix": prefix, "refs": names }),
            "ok",
        ) {
            return Failure::from(e).into_response();
        }
    }

    let (sender, receiver) = tokio::sync::mpsc::channel::<std::io::Result<Bytes>>(8);
    let wants = request.wants;
    tokio::task::spawn_blocking(move || {
        let mut out = Channel {
            sender,
            buffer: Vec::new(),
        };
        let io = |e: std::io::Error| Error::new(ErrorKind::Io, e.to_string());
        let written = out
            .write_all(&answer)
            .map_err(io)
            .and_then(|()| repo.git_pack(&cache, &wants, &common, &mut out))
            .and_then(|_| out.flush().map_err(io));
        if let Err(e) = written {
            // The client sees a pack cut short and fails.
            let _ = out
                .sender
                .blocking_send(Err(std::io::Error::other(e.to_string())));
        }
    });
    let stream = futures_util::stream::unfold(receiver, |mut r| async move {
        r.recv().await.map(|item| (item, r))
    });
    git_response(
        "application/x-git-upload-pack-result",
        Body::from_stream(stream),
    )
}

/// The user, the repository and the ref prefix of a Git request; a
/// response that asks for a password otherwise.
async fn signed_in(
    app: &Arc<App>,
    prefix: &str,
    headers: &HeaderMap,
    from: Option<String>,
) -> Result<(User, Repository, PathBuf, String), Box<Response>> {
    let Mode::Web(web) = &app.mode else {
        return Err(Box::new(text(
            StatusCode::NOT_FOUND,
            "Git is served by apeiron web only",
        )));
    };
    let prefix = prefix.strip_suffix(".git").unwrap_or(prefix).to_owned();
    let valid = !prefix.is_empty()
        && !prefix.starts_with('.')
        && prefix
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'));
    if !valid {
        return Err(Box::new(text(StatusCode::NOT_FOUND, "no such ref prefix")));
    }
    let user = match web.basic_user(headers, from).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            let mut response = text(
                StatusCode::UNAUTHORIZED,
                "sign in with your name and password",
            );
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"Apeiron\", charset=\"UTF-8\""),
            );
            return Err(Box::new(response));
        }
        Err(failure) => return Err(Box::new(failure.into_response())),
    };
    let repo = app
        .repo()
        .map_err(|e| Box::new(Failure::from(e).into_response()))?;
    let cache = cache_path(web, &prefix);
    Ok((user, repo, cache, prefix))
}

/// Brings the Git copy up to date and keeps the refs the user sees.
async fn visible_refs(
    repo: &Repository,
    user: &User,
    cache: &Path,
    prefix: &str,
) -> Result<Vec<GitServedRef>, Error> {
    let repo = repo.clone();
    let request = GitRequest {
        path: cache.to_path_buf(),
        prefix: prefix.to_owned(),
    };
    let refs = blocking(move || {
        let _one = lock(&EXPORTING);
        repo.git_serve_refs(&request)
    })
    .await?;
    Ok(refs
        .into_iter()
        .filter(|r| user.owns(&r.ref_name))
        .collect())
}

/// Where the Git copy of `prefix` lives: `git/<prefix>.git` in the folder
/// of the site's audit log, the server's own state; it can always be made
/// again.
fn cache_path(web: &WebState, prefix: &str) -> PathBuf {
    let state = web.audit.file().parent().unwrap_or(Path::new("."));
    state.join("git").join(format!("{prefix}.git"))
}

/// What a client asks for: the ids it wants, those it has, and whether it
/// said `done`.
struct UploadRequest {
    wants: Vec<String>,
    haves: Vec<String>,
    done: bool,
}

fn parse_request(mut data: &[u8]) -> Option<UploadRequest> {
    let mut request = UploadRequest {
        wants: Vec::new(),
        haves: Vec::new(),
        done: false,
    };
    while data.len() >= 4 {
        let len = usize::from_str_radix(std::str::from_utf8(&data[..4]).ok()?, 16).ok()?;
        if len < 4 {
            // A flush or delimiter packet.
            data = &data[4..];
            continue;
        }
        let line = data.get(4..len)?;
        data = &data[len..];
        let line = std::str::from_utf8(line).ok()?.trim_end_matches('\n');
        let id = |rest: &str| {
            let id = rest.split(' ').next()?;
            (id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| id.to_ascii_lowercase())
        };
        if let Some(rest) = line.strip_prefix("want ") {
            let id = id(rest)?;
            if !request.wants.contains(&id) {
                request.wants.push(id);
            }
        } else if let Some(rest) = line.strip_prefix("have ") {
            request.haves.push(id(rest)?);
        } else if line == "done" {
            request.done = true;
        }
    }
    (!request.wants.is_empty()).then_some(request)
}

/// Appends one packet line: four hex digits of length, then the data.
fn pkt(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(format!("{:04x}", data.len() + 4).as_bytes());
    out.extend_from_slice(data);
}

fn git_response(content_type: &'static str, body: Body) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn text(status: StatusCode, message: &str) -> Response {
    (status, format!("{message}\n")).into_response()
}

/// Sends what is written to the response in pieces of 64 KiB.
struct Channel {
    sender: tokio::sync::mpsc::Sender<std::io::Result<Bytes>>,
    buffer: Vec<u8>,
}

impl Write for Channel {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        if self.buffer.len() >= 64 << 10 {
            self.flush()?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let piece = Bytes::from(std::mem::take(&mut self.buffer));
        self.sender
            .blocking_send(Ok(piece))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the client left"))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_request, pkt};

    #[test]
    fn requests_are_read_from_packet_lines() {
        let id = "a".repeat(40);
        let mut body = Vec::new();
        pkt(&mut body, format!("want {id} agent=git/2\n").as_bytes());
        pkt(&mut body, format!("want {id}\n").as_bytes());
        body.extend_from_slice(b"0000");
        pkt(&mut body, format!("have {}\n", "b".repeat(40)).as_bytes());
        pkt(&mut body, b"done\n");
        let request = parse_request(&body).unwrap();
        assert_eq!(request.wants, [id]);
        assert_eq!(request.haves, ["b".repeat(40)]);
        assert!(request.done);
        assert!(parse_request(b"0000").is_none());
        assert!(parse_request(b"0009want x\n").is_none());
    }
}
