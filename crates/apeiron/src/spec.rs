//! Naming a snapshot (API.md §8.4).

use apeiron_core::{ObjectId, Repository};

use crate::error::{Error, ErrorKind, Result};

const MIN_PREFIX: usize = 8;

/// Resolves a ref name, full id or id prefix, with an optional `~N` suffix.
pub(crate) fn resolve(repo: &Repository, spec: &str) -> Result<ObjectId> {
    let (base, steps) = match spec.rsplit_once('~') {
        Some((base, n)) => {
            let n: u64 = n.parse().map_err(|_| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("invalid snapshot {spec:?}: expected a number after ~"),
                )
            })?;
            (base, n)
        }
        None => (spec, 0),
    };
    let mut id = resolve_base(repo, base)?;
    for step in 0..steps {
        id = repo
            .read_snapshot(&id)?
            .parents
            .first()
            .copied()
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    format!("{base} has only {step} earlier snapshots"),
                )
            })?;
    }
    Ok(id)
}

fn resolve_base(repo: &Repository, base: &str) -> Result<ObjectId> {
    let is_ref_name = apeiron_core::validate_ref_name(base).is_ok();
    if is_ref_name && let Some(id) = repo.read_ref(base)? {
        return Ok(id);
    }
    let is_hex = base.len() >= MIN_PREFIX
        && base
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if is_hex {
        let matches = repo.snapshots_with_prefix(base);
        return match matches.as_slice() {
            [id] => Ok(*id),
            [] => Err(Error::new(
                ErrorKind::NotFound,
                format!("no ref or snapshot {base:?}"),
            )),
            _ => Err(Error::new(
                ErrorKind::Ambiguous,
                format!(
                    "{base:?} matches {} snapshots; use more characters",
                    matches.len()
                ),
            )),
        };
    }
    if is_ref_name {
        return Err(Error::new(ErrorKind::NotFound, format!("no ref {base:?}")));
    }
    Err(Error::new(
        ErrorKind::InvalidInput,
        format!("{base:?} is neither a ref name nor a snapshot id"),
    ))
}
