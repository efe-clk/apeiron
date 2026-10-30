//! The users of the web interface and their rights (USERS.md).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};

use crate::invalid;
use crate::oidc::OidcConfig;

/// Shortest sign-in password (USERS.md §2).
pub const MIN_PASSWORD: usize = 8;

/// What a user may do (USERS.md §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
    Reader,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Reader => "reader",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            "reader" => Ok(Role::Reader),
            _ => Err(invalid(format!(
                "unknown role {s}; use admin, member or reader"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub name: String,
    /// Argon2id hash of the password, as a PHC string; empty when the user
    /// signs in only through the identity provider (IDENTITY.md §2).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    pub role: Role,
    /// Ref names, with `*` at the end for a prefix; not used for `admin`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backups: Vec<String>,
    /// Folders of the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<PathBuf>,
    /// The value of the identity provider's claim that names this user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<String>,
}

impl User {
    /// Whether the user's backups include `name`.
    pub fn owns(&self, name: &str) -> bool {
        self.role == Role::Admin
            || self.backups.iter().any(|b| match b.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => b == name,
            })
    }

    /// Whether `password` is the user's password.
    pub fn verify(&self, password: &str) -> bool {
        !self.password.is_empty()
            && Argon2::default()
                .verify_password(password.as_bytes(), self.password.as_str())
                .is_ok()
    }
}

/// Hashes a sign-in password with Argon2id, as a PHC string.
pub fn hash_password(password: &str) -> Result<String, Error> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(invalid(format!(
            "the password needs at least {MIN_PASSWORD} characters"
        )));
    }
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| Error::new(ErrorKind::Internal, format!("password hash: {e}")))
}

pub(crate) fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// The users file, `web.json` (USERS.md §2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Users {
    /// Signing in through an identity provider (IDENTITY.md §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc: Option<OidcConfig>,
    pub users: Vec<User>,
}

impl Users {
    /// The users in `file`; none if it does not exist.
    pub fn load(file: &Path) -> Result<Users, Error> {
        match fs::read(file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Users::default()),
            Err(e) => Err(Error::new(
                ErrorKind::Io,
                format!("{}: {e}", file.display()),
            )),
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|e| invalid(format!("{}: {e}", file.display()))),
        }
    }

    /// Writes the users to `file`, readable only by its owner. The file is
    /// replaced whole, so the server never reads half of it.
    pub fn save(&self, file: &Path) -> Result<(), Error> {
        self.check()?;
        let data = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let mut tmp = file.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        let written = file
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| crate::web::write_private(&tmp, &data))
            .and_then(|()| fs::rename(&tmp, file));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written.map_err(|e| Error::new(ErrorKind::Io, format!("{}: {e}", file.display())))
    }

    /// Holds `web.json.lock` until the returned file is dropped, so one
    /// change of the users file is made at a time.
    pub fn lock(file: &Path) -> Result<fs::File, Error> {
        let io = |e: io::Error| Error::new(ErrorKind::Io, format!("{}: {e}", file.display()));
        let mut lock = file.as_os_str().to_owned();
        lock.push(".lock");
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock))
            .map_err(io)?;
        lock.lock().map_err(io)?;
        Ok(lock)
    }

    /// Changes the users in `file` with `change` while no one else does
    /// (`web.json.lock`), and saves them if it succeeds.
    pub fn update<T>(
        file: &Path,
        change: impl FnOnce(&mut Users) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let _lock = Users::lock(file)?;
        let mut users = Users::load(file)?;
        let result = change(&mut users)?;
        users.save(file)?;
        Ok(result)
    }

    /// Fails when the identity provider's settings cannot work or two
    /// users have the same identity there.
    pub fn check(&self) -> Result<(), Error> {
        if let Some(config) = &self.oidc {
            config.check()?;
        }
        for (i, user) in self.users.iter().enumerate() {
            let Some(id) = &user.oidc else { continue };
            if let Some(other) = self.users[..i]
                .iter()
                .find(|o| o.oidc.as_ref().is_some_and(|o| o.eq_ignore_ascii_case(id)))
            {
                return Err(invalid(format!(
                    "{} and {} have the same identity {id}",
                    other.name, user.name
                )));
            }
        }
        Ok(())
    }

    /// The user the identity provider names with `value` (IDENTITY.md §3).
    pub fn by_identity(&self, value: &str) -> Option<&User> {
        let config = self.oidc.as_ref()?;
        self.users.iter().find(|u| {
            u.oidc
                .as_deref()
                .is_some_and(|known| config.same(known, value))
        })
    }

    pub fn get(&self, name: &str) -> Option<&User> {
        self.users.iter().find(|u| u.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Result<&mut User, Error> {
        self.users
            .iter_mut()
            .find(|u| u.name == name)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no user named {name}")))
    }

    pub fn add(&mut self, user: User) -> Result<(), Error> {
        if !valid_name(&user.name) {
            return Err(invalid(format!(
                "{}: a user name has 1 to 32 lowercase letters, digits, - and _",
                user.name
            )));
        }
        if self.get(&user.name).is_some() {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("a user named {} already exists", user.name),
            ));
        }
        self.users.push(user);
        Ok(())
    }

    /// Removes a user; the last `admin` stays.
    pub fn remove(&mut self, name: &str) -> Result<(), Error> {
        self.get_mut(name)?;
        let mut left = self.clone();
        left.users.retain(|u| u.name != name);
        left.need_admin()?;
        *self = left;
        Ok(())
    }

    /// Fails unless some user is an `admin`.
    pub fn need_admin(&self) -> Result<(), Error> {
        if self.users.iter().any(|u| u.role == Role::Admin) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Conflict,
                "there must be at least one admin; add one with `apeiron web user add <name> --role admin`",
            ))
        }
    }
}
