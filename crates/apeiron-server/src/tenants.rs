//! Several organizations served by one web server, each at an address of its
//! own with its own repository, users, audit log and share links
//! (TENANTS.md).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use apeiron::{Error, ErrorKind};
use serde::{Deserialize, Serialize};

use crate::invalid;
use crate::users::valid_name;

/// One organization of a server (TENANTS.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    pub name: String,
    /// The host name browsers reach the tenant at, lowercase.
    pub host: String,
    /// The tenant's repository.
    pub repo: PathBuf,
    /// Folders of the server its users may be given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<PathBuf>,
    /// Where the password of an encrypted repository is read from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_file: Option<PathBuf>,
}

impl Tenant {
    /// The folder of the tenant's users file, `web.json`.
    pub fn config_dir(&self) -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("apeiron").join("tenants").join(&self.name))
    }

    /// The folder of the tenant's audit log and share links.
    pub fn state_dir(&self) -> Option<PathBuf> {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|d| d.join("apeiron").join("tenants").join(&self.name))
    }
}

/// A host name: labels of lowercase letters, digits and `-`, joined by
/// dots.
fn valid_host(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// The tenants file, `tenants.json` (TENANTS.md §2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenants {
    pub tenants: Vec<Tenant>,
}

impl Tenants {
    /// `tenants.json` in the configuration folder.
    pub fn default_file() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("apeiron").join("tenants.json"))
    }

    /// The tenants in `file`; none if it does not exist.
    pub fn load(file: &Path) -> Result<Tenants, Error> {
        let tenants: Tenants = match fs::read(file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Tenants::default()),
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Io,
                    format!("{}: {e}", file.display()),
                ));
            }
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|e| invalid(format!("{}: {e}", file.display())))?,
        };
        tenants.check()?;
        Ok(tenants)
    }

    /// Writes the tenants to `file`.
    pub fn save(&self, file: &Path) -> Result<(), Error> {
        self.check()?;
        let data = serde_json::to_vec_pretty(self)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| crate::web::write_private(file, &data))
            .map_err(|e| Error::new(ErrorKind::Io, format!("{}: {e}", file.display())))
    }

    /// Fails when a tenant's name, host or paths cannot work, or two
    /// tenants have the same name or host.
    pub fn check(&self) -> Result<(), Error> {
        for (i, t) in self.tenants.iter().enumerate() {
            if !valid_name(&t.name) {
                return Err(invalid(format!(
                    "{}: a tenant name has 1 to 32 lowercase letters, digits, - and _",
                    t.name
                )));
            }
            if !valid_host(&t.host) {
                return Err(invalid(format!(
                    "{}: a host is a lowercase name such as backup.example.com",
                    t.host
                )));
            }
            let mut paths = std::iter::once(&t.repo)
                .chain(&t.folders)
                .chain(&t.password_file);
            if let Some(p) = paths.find(|p| !p.is_absolute()) {
                return Err(invalid(format!("{} is not a full path", p.display())));
            }
            for other in &self.tenants[..i] {
                if other.name == t.name {
                    return Err(Error::new(
                        ErrorKind::AlreadyExists,
                        format!("a tenant named {} already exists", t.name),
                    ));
                }
                if other.host == t.host {
                    return Err(Error::new(
                        ErrorKind::AlreadyExists,
                        format!(
                            "{} and {} have the same host {}",
                            other.name, t.name, t.host
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<&Tenant, Error> {
        self.tenants
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no tenant named {name}")))
    }

    pub fn add(&mut self, tenant: Tenant) -> Result<(), Error> {
        let mut more = self.clone();
        more.tenants.push(tenant);
        more.check()?;
        *self = more;
        Ok(())
    }

    /// Removes a tenant from the list; its files stay.
    pub fn remove(&mut self, name: &str) -> Result<Tenant, Error> {
        let i = self
            .tenants
            .iter()
            .position(|t| t.name == name)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no tenant named {name}")))?;
        Ok(self.tenants.remove(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(name: &str, host: &str) -> Tenant {
        Tenant {
            name: name.to_owned(),
            host: host.to_owned(),
            // A full path on every system.
            repo: std::env::temp_dir().join(name),
            folders: Vec::new(),
            password_file: None,
        }
    }

    #[test]
    fn names_hosts_and_paths_are_checked() {
        let mut t = Tenants::default();
        t.add(tenant("acme", "acme.example.com")).unwrap();
        assert_eq!(
            t.add(tenant("acme", "other.example.com")).unwrap_err().kind,
            ErrorKind::AlreadyExists
        );
        assert_eq!(
            t.add(tenant("globex", "acme.example.com"))
                .unwrap_err()
                .kind,
            ErrorKind::AlreadyExists
        );
        for host in ["Acme.example.com", "-a.example.com", "a..b", "a_b.com", ""] {
            assert!(t.add(tenant("b", host)).is_err(), "{host}");
        }
        assert!(t.add(tenant("B", "b.example.com")).is_err());
        let mut relative = tenant("c", "c.example.com");
        relative.folders.push(PathBuf::from("data"));
        assert!(t.add(relative).is_err());
        t.add(tenant("globex", "localhost")).unwrap();
        assert_eq!(t.tenants.len(), 2);
        assert_eq!(t.remove("acme").unwrap().host, "acme.example.com");
        assert_eq!(t.remove("acme").unwrap_err().kind, ErrorKind::NotFound);
    }

    #[test]
    fn saved_and_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config/tenants.json");
        assert_eq!(Tenants::load(&file).unwrap(), Tenants::default());
        let mut t = Tenants::default();
        t.add(tenant("acme", "acme.example.com")).unwrap();
        t.save(&file).unwrap();
        assert_eq!(Tenants::load(&file).unwrap(), t);
        fs::write(&file, "{").unwrap();
        assert_eq!(
            Tenants::load(&file).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
}
