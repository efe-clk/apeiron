//! `git clone`, `git fetch` and `git push` over SSH (GIT.md §7.2): the
//! system's SSH server checks the user's key and runs `apeiron git ssh`,
//! which speaks Git's protocol, version 0, on its standard input and output.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

use apeiron::{Error, ErrorKind, GitRequest, Repository};
use serde_json::json;

use crate::audit::{Actor, Audit};
use crate::git::{
    PUSH_LIMIT, advertisement, copy_lock, copy_path, parse_commands, pkt, pushed, report,
    valid_prefix,
};
use crate::users::{Role, User, Users};

/// What one SSH session of Git works with.
pub struct GitSsh {
    pub repo: Repository,
    /// The users file of the web server (USERS.md); the user named in the
    /// key's forced command must be in it.
    pub users: PathBuf,
    pub audit: Audit,
    /// The user the key belongs to.
    pub user: String,
    /// Where the connection came from, for the audit log.
    pub from: Option<String>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn io(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Io, e.to_string())
}

/// The service and the ref prefix of `SSH_ORIGINAL_COMMAND`, such as
/// `git-upload-pack '/git'` or `git-receive-pack 'git.git'`.
fn parse_command(command: &str) -> Result<(bool, String), Error> {
    let (service, path) = command
        .trim()
        .split_once(' ')
        .ok_or_else(|| invalid("only git clone, git fetch and git push are served"))?;
    let push = match service {
        "git-upload-pack" | "git upload-pack" => false,
        "git-receive-pack" | "git receive-pack" => true,
        _ => return Err(invalid("only git clone, git fetch and git push are served")),
    };
    let path = path.trim().trim_matches('\'').trim_matches('"');
    let path = path.trim_start_matches("~/").trim_start_matches('/');
    // `ssh://host/git/<prefix>` names the prefix as the web address does.
    let path = match path.split_once('/') {
        Some(("git", rest)) => rest,
        _ => path,
    };
    let prefix = path.strip_suffix(".git").unwrap_or(path);
    if !valid_prefix(prefix) {
        return Err(Error::new(ErrorKind::NotFound, "no such ref prefix"));
    }
    Ok((push, prefix.to_owned()))
}

/// One packet line: `None` at the end of input, an empty line for a flush.
fn read_pkt(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, Error> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(io(e)),
    }
    let len = std::str::from_utf8(&len)
        .ok()
        .and_then(|l| usize::from_str_radix(l, 16).ok())
        .ok_or_else(|| invalid("not a Git packet"))?;
    if len < 4 {
        return Ok(Some(Vec::new()));
    }
    let mut data = vec![0u8; len - 4];
    input.read_exact(&mut data).map_err(io)?;
    Ok(Some(data))
}

/// The id after `want ` or `have `, lowercase.
fn id_of(rest: &[u8]) -> Result<String, Error> {
    let rest = std::str::from_utf8(rest).map_err(|_| invalid("not a Git request"))?;
    let id = rest.trim_end_matches('\n').split(' ').next().unwrap_or("");
    if id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(id.to_ascii_lowercase())
    } else {
        Err(invalid("not a Git object id"))
    }
}

/// Serves one Git command for `ssh.user` on `input` and `output`.
pub fn serve_git_ssh(
    ssh: &GitSsh,
    command: &str,
    input: impl Read,
    mut output: impl Write,
) -> Result<(), Error> {
    let (push, prefix) = parse_command(command)?;
    let user: User = Users::load(&ssh.users)?
        .get(&ssh.user)
        .cloned()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no user named {}", ssh.user)))?;
    if push && user.role == Role::Reader {
        return Err(invalid("a reader cannot push"));
    }
    let cache = copy_path(&ssh.audit, &prefix);
    let request = GitRequest {
        path: cache.clone(),
        prefix: prefix.clone(),
    };
    let refs: Vec<_> = {
        let _copy = copy_lock(&cache)?;
        ssh.repo.git_serve_refs(&request)?
    }
    .into_iter()
    .filter(|r| user.owns(&r.ref_name))
    .collect();
    let actor = Actor {
        user: Some(user.name.clone()),
        from: ssh.from.clone(),
    };
    output.write_all(&advertisement(&refs, push)).map_err(io)?;
    output.flush().map_err(io)?;
    let mut input = BufReader::new(input);

    if push {
        let mut commands = Vec::new();
        loop {
            match read_pkt(&mut input)? {
                None => return Ok(()),
                Some(line) if line.is_empty() => break,
                Some(line) => pkt(&mut commands, &line),
            }
        }
        if commands.is_empty() {
            return Ok(());
        }
        commands.extend_from_slice(b"0000");
        let (updates, _) = parse_commands(&commands).ok_or_else(|| invalid("not a Git push"))?;
        let deletes_only = updates.iter().all(|u| u.new == "0".repeat(40));
        let pack = if deletes_only {
            Vec::new()
        } else {
            read_pack(&mut input)?
        };
        let results = {
            let _copy = copy_lock(&cache)?;
            ssh.repo
                .git_receive(&request, &updates, &pack, &|name| user.owns(name))
        };
        let (out, results) = report(results, &updates);
        let (refs, moved) = pushed(&updates, &results);
        ssh.audit.record(
            &actor,
            "git_push",
            json!({ "prefix": prefix, "refs": refs, "via": "ssh" }),
            if moved { "ok" } else { "conflict" },
        )?;
        output.write_all(&out).map_err(io)?;
        return output.flush().map_err(io);
    }

    // Wants up to a flush, then haves; the first common commit is
    // acknowledged and ends the talk, as in Git without `multi_ack`.
    let mut wants: Vec<String> = Vec::new();
    loop {
        match read_pkt(&mut input)? {
            None => return Ok(()),
            Some(line) if line.is_empty() => break,
            Some(line) => match line.strip_prefix(b"want ") {
                Some(rest) => {
                    let id = id_of(rest)?;
                    if !refs.iter().any(|r| r.id == id) {
                        return Err(invalid(format!(
                            "{id} is not the tip of a branch or tag you can see"
                        )));
                    }
                    if !wants.contains(&id) {
                        wants.push(id);
                    }
                }
                None => return Err(invalid("shallow clones are not served")),
            },
        }
    }
    if wants.is_empty() {
        return Ok(());
    }
    let mut common: Option<String> = None;
    loop {
        let Some(line) = read_pkt(&mut input)? else {
            return Ok(());
        };
        if line.is_empty() {
            if common.is_none() {
                output.write_all(b"0008NAK\n").map_err(io)?;
                output.flush().map_err(io)?;
            }
        } else if let Some(rest) = line.strip_prefix(b"have ") {
            let id = id_of(rest)?;
            if common.is_none()
                && !ssh
                    .repo
                    .git_common(&cache, &wants, std::slice::from_ref(&id))?
                    .is_empty()
            {
                let mut ack = Vec::new();
                pkt(&mut ack, format!("ACK {id}\n").as_bytes());
                output.write_all(&ack).map_err(io)?;
                common = Some(id);
            }
        } else if line.starts_with(b"done") {
            if common.is_none() {
                output.write_all(b"0008NAK\n").map_err(io)?;
            }
            break;
        } else {
            return Err(invalid("not a Git fetch"));
        }
    }
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| wants.contains(&r.id))
        .map(|r| r.ref_name.as_str())
        .collect();
    ssh.audit.record(
        &actor,
        "git_fetch",
        json!({ "prefix": prefix, "refs": names, "via": "ssh" }),
        "ok",
    )?;
    let common: Vec<String> = common.into_iter().collect();
    ssh.repo.git_pack(&cache, &wants, &common, &mut output)?;
    output.flush().map_err(io)
}

/// Reads exactly one pack, of at most [`PUSH_LIMIT`] bytes: over SSH the
/// client sends it and then waits for the report, so its end is found by
/// reading its objects.
fn read_pack(input: &mut impl BufRead) -> Result<Vec<u8>, Error> {
    let too_large = || invalid("the push is larger than 256 MiB; push fewer commits at a time");
    let mut pack = vec![0u8; 12];
    input.read_exact(&mut pack).map_err(io)?;
    if &pack[..4] != b"PACK" {
        return Err(invalid("not a Git pack"));
    }
    let count = u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]);
    let byte = |input: &mut dyn BufRead, pack: &mut Vec<u8>| -> Result<u8, Error> {
        let mut b = [0u8; 1];
        input.read_exact(&mut b).map_err(io)?;
        pack.push(b[0]);
        Ok(b[0])
    };
    for _ in 0..count {
        let first = byte(input, &mut pack)?;
        let kind = (first >> 4) & 7;
        let mut more = first & 0x80 != 0;
        while more {
            more = byte(input, &mut pack)? & 0x80 != 0;
        }
        match kind {
            // A delta against an object earlier in the pack: its distance.
            6 => while byte(input, &mut pack)? & 0x80 != 0 {},
            // A delta against an object by id.
            7 => {
                for _ in 0..20 {
                    byte(input, &mut pack)?;
                }
            }
            1..=4 => {}
            _ => return Err(invalid("not a Git pack")),
        }
        // The object's zlib stream: read until it ends.
        let mut inflate = flate2::Decompress::new(true);
        let mut sink = [0u8; 32 << 10];
        loop {
            let available = input.fill_buf().map_err(io)?;
            if available.is_empty() {
                return Err(invalid("the pack ends early"));
            }
            let before = inflate.total_in();
            let status = inflate
                .decompress(available, &mut sink, flate2::FlushDecompress::None)
                .map_err(|e| invalid(format!("the pack is damaged: {e}")))?;
            let used = usize::try_from(inflate.total_in() - before).unwrap_or(usize::MAX);
            pack.extend_from_slice(&available[..used]);
            input.consume(used);
            if pack.len() > PUSH_LIMIT {
                return Err(too_large());
            }
            if status == flate2::Status::StreamEnd {
                break;
            }
        }
    }
    let mut trailer = [0u8; 20];
    input.read_exact(&mut trailer).map_err(io)?;
    pack.extend_from_slice(&trailer);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::parse_command;

    #[test]
    fn commands_name_a_service_and_a_prefix() {
        assert_eq!(
            parse_command("git-upload-pack '/git'").unwrap(),
            (false, "git".to_owned())
        );
        assert_eq!(
            parse_command("git-receive-pack 'work.git'").unwrap(),
            (true, "work".to_owned())
        );
        assert_eq!(
            parse_command("git-upload-pack '/git/work.git'").unwrap(),
            (false, "work".to_owned())
        );
        assert!(parse_command("sh -c 'rm -rf /'").is_err());
        assert!(parse_command("git-upload-pack '../etc'").is_err());
    }
}
