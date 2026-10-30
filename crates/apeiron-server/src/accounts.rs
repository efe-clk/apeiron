//! Managing the users of the web interface from its pages (USERS.md §4):
//! an `admin` adds, changes and removes users, as `apeiron web user` does.

use std::path::PathBuf;
use std::sync::Arc;

use apeiron::{Error, ErrorKind};
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::users::{Role, User, Users, hash_password};
use crate::web::{WebState, Who, inside};
use crate::{Answer, App, Mode, blocking, body, invalid, outcome};

/// A user as the pages see it: never the password's hash.
#[derive(Serialize)]
struct View {
    name: String,
    role: Role,
    backups: Vec<String>,
    folders: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oidc: Option<String>,
    password: bool,
}

impl From<&User> for View {
    fn from(u: &User) -> Self {
        View {
            name: u.name.clone(),
            role: u.role,
            backups: u.backups.clone(),
            folders: u.folders.clone(),
            oidc: u.oidc.clone(),
            password: !u.password.is_empty(),
        }
    }
}

/// What the add and change forms send; a field left out stays as it is.
#[derive(Deserialize)]
struct Form {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    role: Option<Role>,
    /// A new password; empty or absent keeps the one there is.
    #[serde(default)]
    password: Option<String>,
    /// Takes the password away, for a user who signs in through the
    /// identity provider.
    #[serde(default)]
    no_password: bool,
    /// The identity at the provider; empty takes it away.
    #[serde(default)]
    oidc: Option<String>,
    #[serde(default)]
    backups: Option<Vec<String>>,
    #[serde(default)]
    folders: Option<Vec<PathBuf>>,
}

/// The audit detail of a user, as `apeiron web user` records it.
fn detail(u: &User) -> Value {
    json!({
        "name": u.name, "role": u.role, "backups": u.backups, "folders": u.folders,
        "oidc": u.oidc,
    })
}

/// The web state and the signed-in user, who must be an `admin`.
fn admin(app: &App, who: Option<Extension<Who>>) -> Result<(&WebState, Who), Error> {
    match (&app.mode, who) {
        (Mode::Web(web), Some(Extension(who))) if who.user.role == Role::Admin => Ok((web, who)),
        (Mode::Web(_), Some(_)) => Err(Error::new(
            ErrorKind::Unsupported,
            "only an admin can manage users",
        )),
        (Mode::Web(_), None) => Err(invalid("sign in first")),
        (Mode::Local { .. }, _) => Err(Error::new(
            ErrorKind::Unsupported,
            "users are only for the web interface",
        )),
    }
}

impl WebState {
    /// Applies `form` to `user`: the checks of USERS.md §2 and §3.
    fn apply(&self, user: &mut User, form: Form) -> Result<(), Error> {
        if let Some(role) = form.role {
            user.role = role;
        }
        if let Some(backups) = form.backups {
            user.backups = backups
                .into_iter()
                .map(|b| b.trim().to_owned())
                .filter(|b| !b.is_empty())
                .collect();
        }
        if let Some(folders) = form.folders {
            user.folders = folders
                .iter()
                .filter(|f| !f.as_os_str().is_empty())
                .map(|f| inside(self.roots(), f))
                .collect::<Result<_, _>>()?;
        }
        if let Some(oidc) = form.oidc {
            let oidc = oidc.trim();
            user.oidc = (!oidc.is_empty()).then(|| oidc.to_owned());
        }
        if form.no_password {
            user.password.clear();
        }
        if let Some(password) = form.password.filter(|p| !p.is_empty()) {
            user.password = hash_password(&password)?;
        }
        if user.password.is_empty() && user.oidc.is_none() {
            return Err(invalid(
                "give the user a password or an identity at the identity provider",
            ));
        }
        Ok(())
    }

    /// Records a change made through the pages.
    fn recorded<T>(
        &self,
        who: &Who,
        event: &str,
        detail: Value,
        result: Result<T, Error>,
    ) -> Result<T, Error> {
        self.audit.record(
            &who.actor(),
            event,
            detail,
            &outcome(result.as_ref().map(|_| ())),
        )?;
        result
    }
}

/// `GET /api/users`: every user and the folders users may be given.
pub(crate) async fn list(State(app): State<Arc<App>>, who: Option<Extension<Who>>) -> Answer {
    let value = blocking(move || {
        let (web, _) = admin(&app, who)?;
        let users = Users::load(web.users_path())?;
        let views: Vec<View> = users.users.iter().map(View::from).collect();
        Ok(json!({"users": views, "folders": web.roots(), "oidc": users.oidc.is_some()}))
    })
    .await?;
    Ok(axum::Json(value))
}

/// `POST /api/users`: adds a user.
pub(crate) async fn add(
    State(app): State<Arc<App>>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let form: Form = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    blocking(move || {
        let (web, who) = admin(&app, who)?;
        let name = form.name.clone().unwrap_or_default();
        let result = Users::update(web.users_path(), |users| {
            let mut user = User {
                name: name.trim().to_owned(),
                password: String::new(),
                role: Role::Member,
                backups: Vec::new(),
                folders: Vec::new(),
                oidc: None,
            };
            web.apply(&mut user, form)?;
            let detail = detail(&user);
            users.add(user)?;
            Ok(detail)
        });
        let shown = result.clone().unwrap_or_else(|_| json!({ "name": name }));
        web.recorded(&who, "user_add", shown, result).map(drop)
    })
    .await?;
    Ok(axum::Json(json!({})))
}

/// `POST /api/users/{name}`: changes a user. An admin cannot take its own
/// admin role away, and there is always an admin left.
pub(crate) async fn change(
    State(app): State<Arc<App>>,
    UrlPath(name): UrlPath<String>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Answer {
    let form: Form = serde_json::from_value(body(&bytes)?).map_err(|e| invalid(e.to_string()))?;
    blocking(move || {
        let (web, who) = admin(&app, who)?;
        let password = form.password.as_ref().is_some_and(|p| !p.is_empty()) || form.no_password;
        let result = Users::update(web.users_path(), |users| {
            if name == who.user.name && form.role.is_some_and(|r| r != Role::Admin) {
                return Err(invalid("you cannot take your own admin role away"));
            }
            let user = users.get_mut(&name)?;
            web.apply(user, form)?;
            let detail = detail(user);
            users.need_admin()?;
            Ok(detail)
        });
        let shown = result.clone().unwrap_or_else(|_| json!({ "name": name }));
        let changed = web.recorded(&who, "user_change", shown, result);
        if password && changed.is_ok() {
            web.audit
                .record(&who.actor(), "user_password", json!({ "name": name }), "ok")?;
        }
        changed.map(drop)
    })
    .await?;
    Ok(axum::Json(json!({})))
}

/// `POST /api/users/{name}/remove`: removes a user other than oneself.
pub(crate) async fn remove(
    State(app): State<Arc<App>>,
    UrlPath(name): UrlPath<String>,
    who: Option<Extension<Who>>,
) -> Answer {
    blocking(move || {
        let (web, who) = admin(&app, who)?;
        let result = if name == who.user.name {
            Err(invalid("you cannot remove yourself"))
        } else {
            Users::update(web.users_path(), |users| users.remove(&name))
        };
        web.recorded(&who, "user_remove", json!({ "name": name }), result)
    })
    .await?;
    Ok(axum::Json(json!({})))
}
