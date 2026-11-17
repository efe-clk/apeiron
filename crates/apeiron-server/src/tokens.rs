//! Access tokens (API.md §2.1): a program calls the JSON API of a web
//! server as one of its users, without a browser and without signing in.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What every token starts with, so one is recognised in a file or a log.
pub const PREFIX: &str = "apt_";

/// One token, as kept in the tokens file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// A name of the user's choosing, unique per user.
    pub name: String,
    /// The user it acts as; it stops working when the user is removed.
    pub user: String,
    /// SHA-256 of the token, hex; the token itself is not kept.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub hash: String,
    pub made: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    tokens: Vec<Token>,
}

fn io_error(path: &Path, e: io::Error) -> Error {
    Error::new(ErrorKind::Io, format!("{}: {e}", path.display()))
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The tokens file: `web-tokens.json` beside the users file.
#[derive(Clone, Debug)]
pub struct Tokens {
    file: PathBuf,
}

impl Tokens {
    /// The tokens of the users in `users_file`.
    pub fn beside(users_file: &Path) -> Self {
        Tokens {
            file: users_file.with_file_name("web-tokens.json"),
        }
    }

    fn read(&self) -> Result<Stored, Error> {
        match fs::read(&self.file) {
            Ok(data) => serde_json::from_slice(&data).map_err(|e| {
                Error::new(ErrorKind::Corrupt, format!("{}: {e}", self.file.display()))
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Stored::default()),
            Err(e) => Err(io_error(&self.file, e)),
        }
    }

    /// Changes the file under a lock and replaces it whole, readable by its
    /// owner only.
    fn change<T>(&self, f: impl FnOnce(&mut Vec<Token>) -> Result<T, Error>) -> Result<T, Error> {
        let path = &self.file;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        }
        let lock_path = path.with_extension("lock");
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_error(&lock_path, e))?;
        lock.lock().map_err(|e| io_error(&lock_path, e))?;
        let mut stored = self.read()?;
        let out = f(&mut stored.tokens)?;
        let data = serde_json::to_vec_pretty(&stored)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let new = path.with_extension("new");
        let write = || -> io::Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut f = options.open(&new)?;
            f.write_all(&data)?;
            f.sync_all()?;
            fs::rename(&new, path)
        };
        write().map_err(|e| io_error(path, e))?;
        Ok(out)
    }

    /// Makes a token named `name` for `user`; returns it with the token,
    /// which is shown only now.
    pub fn add(&self, user: &str, name: &str) -> Result<(Token, String), Error> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "a token's name is one line of 1 to 64 characters",
            ));
        }
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let secret = format!(
            "{PREFIX}{}",
            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let token = Token {
            name: name.to_owned(),
            user: user.to_owned(),
            hash: hash(&secret),
            made: Timestamp::now()
                .round(jiff::Unit::Second)
                .unwrap_or_else(|_| Timestamp::now())
                .to_string(),
        };
        self.change(|tokens| {
            if tokens.iter().any(|t| t.user == user && t.name == name) {
                return Err(Error::new(
                    ErrorKind::AlreadyExists,
                    format!("{user} already has a token named {name}"),
                ));
            }
            tokens.push(token.clone());
            Ok(())
        })?;
        Ok((
            Token {
                hash: String::new(),
                ..token
            },
            secret,
        ))
    }

    /// The tokens, of one user or of all, without their hashes.
    pub fn list(&self, user: Option<&str>) -> Result<Vec<Token>, Error> {
        Ok(self
            .read()?
            .tokens
            .into_iter()
            .filter(|t| user.is_none_or(|u| t.user == u))
            .map(|t| Token {
                hash: String::new(),
                ..t
            })
            .collect())
    }

    /// Removes the token `name` of `user`; it stops working at once.
    pub fn remove(&self, user: &str, name: &str) -> Result<(), Error> {
        self.change(|tokens| {
            let before = tokens.len();
            tokens.retain(|t| !(t.user == user && t.name == name));
            if tokens.len() == before {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("{user} has no token named {name}"),
                ));
            }
            Ok(())
        })
    }

    /// The token `secret` stands for, if any.
    pub fn find(&self, secret: &str) -> Option<Token> {
        if !secret.starts_with(PREFIX) {
            return None;
        }
        let wanted = hash(secret);
        // The file is read at each request, so a removed token stops at
        // once; comparing hashes of a secret leaks nothing of it.
        self.read()
            .ok()?
            .tokens
            .into_iter()
            .find(|t| t.hash == wanted)
    }
}
