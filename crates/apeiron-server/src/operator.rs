//! Managing the tenants from the pages (TENANTS.md §4.1): an `admin` of an
//! operator tenant lists, adds, limits and removes tenants while the server
//! runs, as `apeiron web tenant` does.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};

use apeiron::{Error, ErrorKind, Repository};
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::{Extension, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::audit::{Audit, Syslog};
use crate::shares::Shares;
use crate::tenants::{Tenant, Tenants};
use crate::users::{Role, User, Users, hash_password};
use crate::web::{Limits, Site, WebState, Who, site};
use crate::{Answer, App, Mode, blocking, body, host_name, invalid, outcome, router};

/// What the server needs to add tenants while it runs.
#[derive(Clone, Debug)]
pub struct Manage {
    /// `tenants.json`, changed as `apeiron web tenant` changes it.
    pub file: PathBuf,
    /// The configuration and state folders, such as `~/.config/apeiron`
    /// and `~/.local/state/apeiron`; a tenant's files are under
    /// `tenants/<name>` in them (TENANTS.md §2).
    pub config: PathBuf,
    pub state: PathBuf,
    pub standby: bool,
    /// Where the audit events of new tenants are copied (AUDIT.md §5).
    pub syslog: Option<Syslog>,
}

/// A tenant being served.
pub(crate) struct Hosted {
    pub(crate) name: String,
    pub(crate) app: Arc<App>,
    pub(crate) router: Router,
}

/// The tenants being served, by host.
pub(crate) struct Registry {
    pub(crate) manage: Option<Manage>,
    pub(crate) sites: RwLock<HashMap<String, Hosted>>,
}

impl Registry {
    /// The router of the tenant at `host`, if one is served there.
    pub(crate) fn router(&self, host: &str) -> Option<Router> {
        self.sites
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&host_name(host))
            .map(|h| h.router.clone())
    }
}

/// The files of `tenant` (TENANTS.md §2): users, audit log and share links.
fn tenant_files(tenant: &Tenant, manage: &Manage) -> (PathBuf, Audit, Shares) {
    let config = manage.config.join("tenants").join(&tenant.name);
    let state = manage.state.join("tenants").join(&tenant.name);
    let mut audit = Audit::new(state.join("audit.log"));
    if let Some(syslog) = &manage.syslog {
        audit = audit.copying_to(syslog.clone(), Some(&tenant.name));
    }
    (
        config.join("web.json"),
        audit,
        Shares::new(state.join("shares.json")),
    )
}

/// A tenant as the pages see it.
#[derive(Serialize)]
struct View {
    #[serde(flatten)]
    tenant: Tenant,
    /// Whether the server serves it now; one added by the commands is
    /// served after a restart.
    serving: bool,
}

/// What the add form sends.
#[derive(Deserialize)]
struct AddForm {
    name: String,
    host: String,
    repo: PathBuf,
    #[serde(default)]
    folders: Vec<PathBuf>,
    #[serde(default)]
    password_file: Option<PathBuf>,
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    max_jobs: Option<u32>,
    /// The first admin, when the tenant has none yet.
    #[serde(default)]
    admin: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// What the limits form sends; 0 lifts a limit, absent keeps it.
#[derive(Deserialize)]
struct LimitForm {
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    max_jobs: Option<u32>,
}

/// The web state, the signed-in user, who must be an `admin` of an operator
/// tenant, and the tenants being served.
fn operator(
    app: &App,
    who: Option<Extension<Who>>,
) -> Result<(&WebState, Who, Arc<Registry>, Manage), Error> {
    let refused = || {
        Error::new(
            ErrorKind::Unsupported,
            "only an admin of the operator tenant manages tenants",
        )
    };
    let Mode::Web(web) = &app.mode else {
        return Err(refused());
    };
    let Some(Extension(who)) = who else {
        return Err(invalid("sign in first"));
    };
    if who.user.role != Role::Admin {
        return Err(refused());
    }
    let registry = web.registry().ok_or_else(refused)?;
    let manage = registry.manage.clone().ok_or_else(refused)?;
    Ok((web, who, registry, manage))
}

/// Records a change in the operator's log.
fn recorded<T>(
    web: &WebState,
    who: &Who,
    event: &str,
    detail: Value,
    result: Result<T, Error>,
) -> Result<T, Error> {
    web.audit.record(
        &who.actor(),
        event,
        detail,
        &outcome(result.as_ref().map(|_| ())),
    )?;
    result
}

/// `GET /api/tenants`: every tenant of `tenants.json`.
pub(crate) async fn list(State(app): State<Arc<App>>, who: Option<Extension<Who>>) -> Answer {
    let value = blocking(move || {
        let (_, _, registry, manage) = operator(&app, who)?;
        let tenants = Tenants::load(&manage.file)?;
        let sites = registry
            .sites
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let views: Vec<View> = tenants
            .tenants
            .into_iter()
            .map(|t| View {
                serving: sites.values().any(|h| h.name == t.name),
                tenant: t,
            })
            .collect();
        Ok(json!({ "tenants": views }))
    })
    .await?;
    Ok(axum::Json(value))
}

/// Opens the repository of `tenant`, making it when the folder does not
/// exist; an encrypted one needs its password file.
fn open_repo(tenant: &Tenant) -> Result<Repository, Error> {
    if !tenant.repo.exists() {
        return Repository::init(&tenant.repo);
    }
    match (Repository::open(&tenant.repo), &tenant.password_file) {
        (Err(e), Some(file)) if e.kind == ErrorKind::Key => {
            let text = std::fs::read_to_string(file)
                .map_err(|e| Error::new(ErrorKind::Io, format!("{}: {e}", file.display())))?;
            let secret = text.lines().next().unwrap_or_default();
            Repository::unlock(&tenant.repo, secret)
        }
        (Err(e), None) if e.kind == ErrorKind::Key => Err(Error::new(
            ErrorKind::Key,
            "the repository is encrypted; give the file its password is in",
        )),
        (result, _) => result,
    }
}

/// `POST /api/tenants`: adds a tenant and serves it at once.
pub(crate) async fn add(
    State(app): State<Arc<App>>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let form: AddForm =
        serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    blocking(move || {
        let (web, who, registry, manage) = operator(&app, who)?;
        let full = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
        let tenant = Tenant {
            name: form.name.trim().to_owned(),
            host: form.host.trim().to_ascii_lowercase(),
            repo: full(form.repo),
            folders: form.folders.into_iter().map(full).collect(),
            password_file: form.password_file.map(full),
            replicas: Vec::new(),
            max_bytes: form.max_bytes.filter(|&n| n > 0),
            max_jobs: form.max_jobs.filter(|&n| n > 0),
            operator: false,
        };
        let detail = serde_json::to_value(&tenant).unwrap_or_default();
        let result = (|| {
            let mut tenants = Tenants::load(&manage.file)?;
            tenants.add(tenant.clone())?;
            let (users, audit, shares) = tenant_files(&tenant, &manage);
            let repo = open_repo(&tenant)?;
            Users::update(&users, |list| {
                if list.users.iter().any(|u| u.role == Role::Admin) {
                    return Ok(());
                }
                let name = form.admin.as_deref().map(str::trim).unwrap_or_default();
                let password = form.password.as_deref().unwrap_or_default();
                if name.is_empty() || password.is_empty() {
                    return Err(invalid("give the tenant's first admin a name and password"));
                }
                list.add(User {
                    name: name.to_owned(),
                    password: hash_password(password)?,
                    role: Role::Admin,
                    backups: Vec::new(),
                    folders: Vec::new(),
                    oidc: None,
                    totp: None,
                })
            })?;
            let served = site(Site {
                repo,
                users,
                folders: tenant.folders.clone(),
                audit: audit.clone(),
                shares,
                replicas: Vec::new(),
                standby: manage.standby,
                limits: Limits {
                    max_bytes: tenant.max_bytes,
                    max_jobs: tenant.max_jobs,
                },
                jobs: Some(tenant.jobs(&manage.config, &manage.state)),
            })?;
            tenants.save(&manage.file)?;
            audit.record(&who.actor(), "tenant_add", detail.clone(), "ok")?;
            registry
                .sites
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(
                    host_name(&tenant.host),
                    Hosted {
                        name: tenant.name.clone(),
                        router: router(served.clone()),
                        app: served,
                    },
                );
            Ok(())
        })();
        recorded(web, &who, "tenant_add", detail, result)
    })
    .await?;
    Ok(axum::Json(json!({})))
}

/// `POST /api/tenants/{name}/limits`: sets or lifts the limits of a
/// tenant; a served one gets them at once.
pub(crate) async fn limit(
    State(app): State<Arc<App>>,
    UrlPath(name): UrlPath<String>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let form: LimitForm =
        serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    blocking(move || {
        let (web, who, registry, manage) = operator(&app, who)?;
        let result = (|| {
            let mut tenants = Tenants::load(&manage.file)?;
            let tenant = tenants.get_mut(&name)?;
            if let Some(n) = form.max_bytes {
                tenant.max_bytes = (n > 0).then_some(n);
            }
            if let Some(n) = form.max_jobs {
                tenant.max_jobs = (n > 0).then_some(n);
            }
            let tenant = tenant.clone();
            tenants.save(&manage.file)?;
            let detail = json!({
                "name": tenant.name,
                "max_bytes": tenant.max_bytes,
                "max_jobs": tenant.max_jobs,
            });
            let sites = registry
                .sites
                .read()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(Mode::Web(served)) = sites
                .values()
                .find(|h| h.name == tenant.name)
                .map(|h| &h.app.mode)
            {
                served.set_limits(Limits {
                    max_bytes: tenant.max_bytes,
                    max_jobs: tenant.max_jobs,
                });
                served
                    .audit
                    .record(&who.actor(), "tenant_limit", detail.clone(), "ok")?;
            }
            Ok(detail)
        })();
        let shown = result.clone().unwrap_or_else(|_| json!({ "name": name }));
        recorded(web, &who, "tenant_limit", shown, result).map(drop)
    })
    .await?;
    Ok(axum::Json(json!({})))
}

/// `POST /api/tenants/{name}/remove`: stops serving a tenant other than
/// the operator's own; its repository and files stay.
pub(crate) async fn remove(
    State(app): State<Arc<App>>,
    UrlPath(name): UrlPath<String>,
    who: Option<Extension<Who>>,
) -> Answer {
    blocking(move || {
        let (web, who, registry, manage) = operator(&app, who)?;
        let result = (|| {
            let mut sites = registry
                .sites
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if sites
                .values()
                .any(|h| h.name == name && Arc::ptr_eq(&h.app, &app))
            {
                return Err(invalid("the operator tenant cannot remove itself"));
            }
            let mut tenants = Tenants::load(&manage.file)?;
            tenants.remove(&name)?;
            tenants.save(&manage.file)?;
            if let Some(Mode::Web(served)) =
                sites.values().find(|h| h.name == name).map(|h| &h.app.mode)
            {
                served.audit.record(
                    &who.actor(),
                    "tenant_remove",
                    json!({ "name": name }),
                    "ok",
                )?;
            }
            sites.retain(|_, h| h.name != name);
            Ok(())
        })();
        recorded(web, &who, "tenant_remove", json!({ "name": name }), result)
    })
    .await?;
    Ok(axum::Json(json!({})))
}
