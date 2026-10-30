//! The users of the web interface and their rights (USERS.md).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};

use crate::invalid;

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
    /// Argon2id hash of the password, as a PHC string.
    pub password: String,
    pub role: Role,
    /// Ref names, with `*` at the end for a prefix; not used for `admin`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backups: Vec<String>,
    /// Folders of the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<PathBuf>,
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
        Argon2::default()
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

fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// The users file, `web.json` (USERS.md §2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Users {
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

    /// Writes the users to `file`, readable only by its owner.
    pub fn save(&self, file: &Path) -> Result<(), Error> {
        let data = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| crate::web::write_private(file, &data))
            .map_err(|e| Error::new(ErrorKind::Io, format!("{}: {e}", file.display())))
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
