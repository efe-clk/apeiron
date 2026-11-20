//! SCIM 2.0 (USERS.md §4.1): an organization's identity provider makes,
//! changes, turns off and removes the users of a web server through
//! `/scim/v2/Users`, with an `admin`'s access token.

use std::sync::Arc;

use apeiron::{Error, ErrorKind};
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::users::{Role, User, Users};
use crate::web::{WebState, Who, free_name};
use crate::{App, Mode, blocking, outcome};

const USER: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const LIST: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const ERROR: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
const PATCH: &str = "urn:ietf:params:scim:api:messages:2.0:PatchOp";
const CONFIG: &str = "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig";
const CONTENT_TYPE: &str = "application/scim+json";

fn answer(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, CONTENT_TYPE)],
        body.to_string(),
    )
        .into_response()
}

fn failure(status: StatusCode, detail: impl ToString) -> Response {
    answer(
        status,
        json!({"schemas": [ERROR], "status": status.as_u16().to_string(), "detail": detail.to_string()}),
    )
}

fn from_error(e: &Error) -> Response {
    let status = match e.kind {
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::AlreadyExists => StatusCode::CONFLICT,
        ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    failure(status, &e.message)
}

/// The web server and the admin a request comes from, or the answer that
/// refuses it.
fn admin(
    app: &App,
    who: Option<Extension<Who>>,
) -> Result<(&WebState, Who), (StatusCode, &'static str)> {
    match (&app.mode, who) {
        (Mode::Web(web), Some(Extension(who))) if who.user.role == Role::Admin => Ok((web, who)),
        (Mode::Web(_), Some(_)) => Err((StatusCode::FORBIDDEN, "only an admin's token")),
        (Mode::Web(_), None) => Err((StatusCode::UNAUTHORIZED, "an admin's token is needed")),
        (Mode::Local { .. }, _) => Err((StatusCode::NOT_FOUND, "not a web server")),
    }
}

fn resource(user: &User) -> Value {
    json!({
        "schemas": [USER],
        "id": user.name,
        "userName": user.oidc.clone().unwrap_or_else(|| user.name.clone()),
        "active": !user.disabled,
        "meta": {"resourceType": "User", "location": format!("/scim/v2/Users/{}", user.name)},
    })
}

/// `true`, `false`, or the same as text, as some providers send it.
fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s.eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

/// Runs `change` on the users file and records `event` in the audit log.
fn changed(
    web: &WebState,
    who: &Who,
    event: &str,
    change: impl FnOnce(&mut Users) -> Result<Option<User>, Error>,
) -> Result<Option<User>, Error> {
    let result = Users::update(web.users_path(), change);
    let detail = match &result {
        Ok(Some(user)) => json!({"name": user.name, "oidc": user.oidc, "active": !user.disabled}),
        _ => json!({}),
    };
    web.audit.record(
        &who.actor(),
        event,
        detail,
        &outcome(result.as_ref().map(drop)),
    )?;
    result
}

pub(crate) async fn config() -> Response {
    let off = json!({"supported": false});
    answer(
        StatusCode::OK,
        json!({
            "schemas": [CONFIG],
            "patch": {"supported": true},
            "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
            "filter": {"supported": true, "maxResults": 200},
            "changePassword": off, "sort": off, "etag": off,
            "authenticationSchemes": [{
                "type": "oauthbearertoken", "name": "Access token",
                "description": "An access token of an admin (apeiron web token add)",
            }],
        }),
    )
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    filter: Option<String>,
    #[serde(rename = "startIndex")]
    start_index: Option<usize>,
    count: Option<usize>,
}

/// The value of a filter `userName eq "value"`, the one filter providers
/// use to find a user.
fn user_name_filter(filter: &str) -> Option<String> {
    let mut parts = filter.splitn(3, ' ');
    let (attribute, op, value) = (parts.next()?, parts.next()?, parts.next()?);
    (attribute.eq_ignore_ascii_case("userName") && op.eq_ignore_ascii_case("eq"))
        .then(|| value.trim().trim_matches('"').to_owned())
}

pub(crate) async fn list(
    State(app): State<Arc<App>>,
    who: Option<Extension<Who>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let web = match admin(&app, who) {
        Ok((web, _)) => web,
        Err((status, why)) => return failure(status, why),
    };
    let wanted = match q.filter.as_deref().map(user_name_filter) {
        Some(None) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "only userName eq \"…\" is supported",
            );
        }
        Some(Some(value)) => Some(value),
        None => None,
    };
    let users = match Users::load(web.users_path()) {
        Ok(users) => users,
        Err(e) => return from_error(&e),
    };
    let found: Vec<&User> = users
        .users
        .iter()
        .filter(|u| match &wanted {
            Some(value) => u
                .oidc
                .as_deref()
                .is_some_and(|known| users.same_identity(known, value)),
            None => true,
        })
        .collect();
    let start = q.start_index.unwrap_or(1).max(1);
    let count = q.count.unwrap_or(100).min(200);
    let page: Vec<Value> = found
        .iter()
        .skip(start - 1)
        .take(count)
        .map(|u| resource(u))
        .collect();
    answer(
        StatusCode::OK,
        json!({
            "schemas": [LIST], "totalResults": found.len(), "startIndex": start,
            "itemsPerPage": page.len(), "Resources": page,
        }),
    )
}

pub(crate) async fn get(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<String>,
    who: Option<Extension<Who>>,
) -> Response {
    let web = match admin(&app, who) {
        Ok((web, _)) => web,
        Err((status, why)) => return failure(status, why),
    };
    match Users::load(web.users_path()) {
        Ok(users) => match users.get(&id) {
            Some(user) => answer(StatusCode::OK, resource(user)),
            None => failure(StatusCode::NOT_FOUND, format!("no user {id}")),
        },
        Err(e) => from_error(&e),
    }
}

/// `POST /scim/v2/Users`: a user who signs in through the provider, as a
/// `member` with the backups `backup/<name>-*`.
pub(crate) async fn add(
    State(app): State<Arc<App>>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Response {
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::BAD_REQUEST, e),
    };
    let who = match admin(&app, who) {
        Ok((_, who)) => who,
        Err((status, why)) => return failure(status, why),
    };
    let result = blocking(move || {
        let web = web_of(&app)?;
        let value = body["userName"]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("userName is missing"))?
            .to_owned();
        let active = body.get("active").and_then(flag).unwrap_or(true);
        changed(web, &who, "scim_user_add", |users| {
            if users.users.iter().any(|u| {
                u.oidc
                    .as_deref()
                    .is_some_and(|k| users.same_identity(k, &value))
            }) {
                return Err(Error::new(
                    ErrorKind::AlreadyExists,
                    format!("a user is already {value}"),
                ));
            }
            let name = free_name(users, &value);
            let user = User {
                backups: vec![format!("backup/{name}-*")],
                name,
                password: String::new(),
                role: Role::Member,
                folders: Vec::new(),
                oidc: Some(value),
                totp: None,
                from_groups: false,
                max_bytes: None,
                disabled: !active,
            };
            users.add(user.clone())?;
            Ok(Some(user))
        })
    })
    .await;
    match result {
        Ok(Some(user)) => answer(StatusCode::CREATED, resource(&user)),
        Ok(None) => failure(StatusCode::INTERNAL_SERVER_ERROR, "no user"),
        Err(e) => from_error(&e),
    }
}

fn web_of(app: &App) -> Result<&WebState, Error> {
    match &app.mode {
        Mode::Web(web) => Ok(web),
        Mode::Local { .. } => Err(invalid("not a web server")),
    }
}

/// Applies `active` and `userName` from a PUT body or a PATCH operation.
fn apply(user: &mut User, attribute: &str, value: &Value) -> Result<(), Error> {
    match attribute.to_ascii_lowercase().as_str() {
        "active" => {
            user.disabled = !flag(value).ok_or_else(|| invalid("active is true or false"))?;
        }
        "username" => {
            let v = value.as_str().filter(|v| !v.trim().is_empty());
            user.oidc = Some(
                v.ok_or_else(|| invalid("userName is text"))?
                    .trim()
                    .to_owned(),
            );
        }
        // Names, e-mail addresses and the like are kept by the provider.
        _ => {}
    }
    Ok(())
}

async fn change(
    app: Arc<App>,
    id: String,
    who: Option<Extension<Who>>,
    edit: impl FnOnce(&mut User) -> Result<(), Error> + Send + 'static,
) -> Response {
    let who = match admin(&app, who) {
        Ok((_, who)) => who,
        Err((status, why)) => return failure(status, why),
    };
    let result = blocking(move || {
        changed(web_of(&app)?, &who, "scim_user_change", |users| {
            let user = users.get_mut(&id)?;
            edit(user)?;
            let user = user.clone();
            users.need_admin()?;
            Ok(Some(user))
        })
    })
    .await;
    match result {
        Ok(Some(user)) => answer(StatusCode::OK, resource(&user)),
        Ok(None) => failure(StatusCode::INTERNAL_SERVER_ERROR, "no user"),
        Err(e) => from_error(&e),
    }
}

pub(crate) async fn put(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<String>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Response {
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::BAD_REQUEST, e),
    };
    change(app, id, who, move |user| {
        for attribute in ["userName", "active"] {
            if let Some(value) = body.get(attribute) {
                apply(user, attribute, value)?;
            }
        }
        Ok(())
    })
    .await
}

pub(crate) async fn patch(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<String>,
    who: Option<Extension<Who>>,
    bytes: Bytes,
) -> Response {
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::BAD_REQUEST, e),
    };
    if !body["schemas"]
        .as_array()
        .is_some_and(|s| s.iter().any(|v| v == PATCH))
    {
        return failure(StatusCode::BAD_REQUEST, "not a PatchOp");
    }
    change(app, id, who, move |user| {
        for op in body["Operations"].as_array().map_or(&[][..], Vec::as_slice) {
            let kind = op["op"].as_str().unwrap_or_default().to_ascii_lowercase();
            if kind != "replace" && kind != "add" {
                return Err(invalid(format!("the operation {kind:?} is not supported")));
            }
            match op["path"].as_str() {
                Some(path) => apply(user, path, &op["value"])?,
                None => {
                    for (attribute, value) in op["value"].as_object().into_iter().flatten() {
                        apply(user, attribute, value)?;
                    }
                }
            }
        }
        Ok(())
    })
    .await
}

pub(crate) async fn remove(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<String>,
    who: Option<Extension<Who>>,
) -> Response {
    let who = match admin(&app, who) {
        Ok((_, who)) => who,
        Err((status, why)) => return failure(status, why),
    };
    let result = blocking(move || {
        changed(web_of(&app)?, &who, "scim_user_remove", |users| {
            let user = users
                .get(&id)
                .cloned()
                .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no user {id}")))?;
            users.remove(&id)?;
            users.need_admin()?;
            Ok(Some(user))
        })
    })
    .await;
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => from_error(&e),
    }
}
