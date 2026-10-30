//! Signing in through an identity provider (IDENTITY.md), against a small
//! provider made for the test.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use apeiron::Repository;
use apeiron_server::{Audit, OidcConfig, Role, Shares, User, Users, Web, WebOptions};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RSA_PKCS1_SHA256, RsaKeyPair,
};
use axum::extract::State;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const CLIENT: &str = "apeiron";
const SECRET: &str = "s3cret";

/// What the test provider knows.
struct Idp {
    issuer: String,
    key: EcdsaKeyPair,
    rsa: RsaKeyPair,
    /// Codes handed out: the nonce and the PKCE challenge of each.
    codes: HashMap<String, (String, String)>,
    /// Claims put into the next ID token over the usual ones.
    next: Value,
    /// Signs the next token with this other key instead.
    forge: Option<EcdsaKeyPair>,
    /// The `alg` of the next token's header.
    alg: String,
}

fn b64(data: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

fn new_key() -> EcdsaKeyPair {
    EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap()
}

impl Idp {
    fn token(&self, nonce: &str) -> String {
        let kid = if self.alg == "RS256" { "k2" } else { "k1" };
        let header = json!({"alg": self.alg, "kid": kid, "typ": "JWT"});
        let now = jiff::Timestamp::now().as_second();
        let mut claims = json!({
            "iss": self.issuer, "aud": CLIENT, "sub": "123", "exp": now + 300,
            "iat": now, "nonce": nonce, "email": "ayse@example.com", "email_verified": true,
        });
        for (k, v) in self.next.as_object().unwrap() {
            claims[k] = v.clone();
        }
        let signed = format!("{}.{}", b64(header.to_string()), b64(claims.to_string()));
        let signature = if self.alg == "RS256" {
            let mut out = vec![0; self.rsa.public_modulus_len()];
            let rng = SystemRandom::new();
            self.rsa
                .sign(&RSA_PKCS1_SHA256, &rng, signed.as_bytes(), &mut out)
                .unwrap();
            out
        } else {
            let key = self.forge.as_ref().unwrap_or(&self.key);
            let signature = key.sign(&SystemRandom::new(), signed.as_bytes()).unwrap();
            signature.as_ref().to_vec()
        };
        format!("{signed}.{}", b64(signature))
    }
}

type Shared = Arc<Mutex<Idp>>;

async fn discovery(State(idp): State<Shared>) -> axum::Json<Value> {
    let issuer = idp.lock().unwrap().issuer.clone();
    axum::Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/keys"),
    }))
}

async fn keys(State(idp): State<Shared>) -> axum::Json<Value> {
    let idp = idp.lock().unwrap();
    let point = idp.key.public_key().as_ref().to_vec();
    let rsa = idp.rsa.public_key();
    axum::Json(json!({"keys": [{
        "kty": "EC", "crv": "P-256", "kid": "k1", "use": "sig",
        "x": b64(&point[1..33]), "y": b64(&point[33..65]),
    }, {
        "kty": "RSA", "kid": "k2", "alg": "RS256",
        "n": b64(rsa.modulus().big_endian_without_leading_zero()),
        "e": b64(rsa.exponent().big_endian_without_leading_zero()),
    }]}))
}

async fn token(
    State(idp): State<Shared>,
    headers: axum::http::HeaderMap,
    body: String,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    let refuse = |e: &str| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": e})),
        )
    };
    let form: HashMap<String, String> = form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT}:{SECRET}"))
    );
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(basic.as_str()) {
        return refuse("invalid_client");
    }
    let mut idp = idp.lock().unwrap();
    let Some((nonce, challenge)) = idp.codes.remove(&form["code"]) else {
        return refuse("invalid_grant");
    };
    if b64(Sha256::digest(form["code_verifier"].as_bytes())) != challenge {
        return refuse("invalid_grant");
    }
    let id_token = idp.token(&nonce);
    (
        axum::http::StatusCode::OK,
        axum::Json(json!({"id_token": id_token})),
    )
}

/// Starts the provider on a thread of its own.
fn provider() -> Shared {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let idp = Arc::new(Mutex::new(Idp {
        issuer,
        key: new_key(),
        rsa: RsaKeyPair::generate(KeySize::Rsa2048).unwrap(),
        codes: HashMap::new(),
        next: json!({}),
        forge: None,
        alg: "ES256".to_owned(),
    }));
    let state = idp.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let app = axum::Router::new()
                .route("/.well-known/openid-configuration", get(discovery))
                .route("/keys", get(keys))
                .route("/token", post(token))
                .with_state(state);
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    idp
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn cookies(&self) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .map(|(_, v)| v.split(';').next().unwrap())
            .collect()
    }
}

fn send(address: SocketAddr, method: &str, path: &str, cookie: &str, body: &str) -> Reply {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nCookie: {cookie}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap()[9..12].parse().unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(n, v)| (n.to_lowercase(), v.to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

/// The query of a URL as a map.
fn query(url: &str) -> HashMap<String, String> {
    let query = url.split_once('?').map_or("", |(_, q)| q);
    form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect()
}

fn start(dir: &Path, idp: &Shared) -> (Web, SocketAddr, std::path::PathBuf) {
    let file = dir.join("config/web.json");
    let issuer = idp.lock().unwrap().issuer.clone();
    let users = Users {
        oidc: Some(OidcConfig {
            issuer,
            client_id: CLIENT.to_owned(),
            client_secret: Some(SECRET.to_owned()),
            address: "https://backup.example.com/".to_owned(),
            claim: "email".to_owned(),
        }),
        users: vec![
            User {
                name: "root".to_owned(),
                password: apeiron_server::hash_password("correct horse").unwrap(),
                role: Role::Admin,
                backups: Vec::new(),
                folders: Vec::new(),
                oidc: None,
            },
            User {
                name: "ayse".to_owned(),
                password: String::new(),
                role: Role::Member,
                backups: vec!["backup/ayse-*".to_owned()],
                folders: Vec::new(),
                oidc: Some("Ayse@Example.com".to_owned()),
            },
        ],
    };
    users.save(&file).unwrap();
    fs::create_dir_all(dir.join("home")).unwrap();
    let server = Web::start(WebOptions {
        repo: Repository::init(dir.join("repo")).unwrap(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        users: file.clone(),
        folders: vec![dir.join("home")],
        audit: Audit::new(dir.join("audit.log")),
        shares: Shares::new(dir.join("shares.json")),
        replicas: Vec::new(),
        standby: false,
    })
    .unwrap();
    let address = server.address();
    (server, address, file)
}

/// Goes through a sign-in as a browser would; `change` may alter what the
/// provider is asked before the browser comes back. Returns the reply to
/// the callback.
fn sign_in(address: SocketAddr, idp: &Shared, change: impl FnOnce(&mut Idp, &mut String)) -> Reply {
    let started = send(address, "GET", "/api/oidc/start", "", "");
    assert_eq!(started.status, 302, "{}", started.body);
    let to = started.header("location").unwrap().to_owned();
    let asked = query(&to);
    let issuer = idp.lock().unwrap().issuer.clone();
    assert!(to.starts_with(&format!("{issuer}/authorize?")), "{to}");
    assert_eq!(asked["client_id"], CLIENT);
    assert_eq!(
        asked["redirect_uri"],
        "https://backup.example.com/api/oidc/callback"
    );
    assert_eq!(asked["code_challenge_method"], "S256");
    let cookie = started.cookies()[0].to_owned();
    assert_eq!(cookie, format!("apeiron_oidc={}", asked["state"]));
    let code = format!("code-{}", asked["state"]);
    let mut state = asked["state"].clone();
    {
        let mut idp = idp.lock().unwrap();
        idp.codes.insert(
            code.clone(),
            (asked["nonce"].clone(), asked["code_challenge"].clone()),
        );
        change(&mut idp, &mut state);
    }
    let reply = send(
        address,
        "GET",
        &format!("/api/oidc/callback?code={code}&state={state}"),
        &cookie,
        "",
    );
    let mut idp = idp.lock().unwrap();
    idp.next = json!({});
    idp.forge = None;
    idp.alg = "ES256".to_owned();
    reply
}

fn session_cookie(reply: &Reply) -> Option<String> {
    reply
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("apeiron_session=") && c.len() > "apeiron_session=".len())
        .map(str::to_owned)
}

#[test]
fn signing_in_through_the_provider() {
    let dir = tempfile::tempdir().unwrap();
    let idp = provider();
    let (_server, address, file) = start(dir.path(), &idp);

    // The sign-in screen offers the provider.
    let session = send(address, "GET", "/api/session", "", "");
    assert_eq!(
        serde_json::from_str::<Value>(&session.body).unwrap()["oidc"],
        true
    );

    // A sign-in that works starts a session of the user it names.
    let done = sign_in(address, &idp, |_, _| {});
    assert_eq!(done.status, 302);
    assert_eq!(done.header("location"), Some("/"));
    let cookie = session_cookie(&done).expect("a session");
    let session = send(address, "GET", "/api/session", &cookie, "");
    assert_eq!(
        serde_json::from_str::<Value>(&session.body).unwrap()["user"],
        json!({"name": "ayse", "role": "member"})
    );

    // RSA keys work too.
    let rsa = sign_in(address, &idp, |i, _| i.alg = "RS256".to_owned());
    assert!(session_cookie(&rsa).is_some(), "{:?}", rsa.headers);

    // A user without a password cannot sign in with one.
    let login = send(
        address,
        "POST",
        "/api/login",
        "",
        r#"{"name": "ayse", "password": ""}"#,
    );
    assert_eq!(login.status, 401);

    // Every way a token can be wrong is refused.
    let refused = |reply: Reply| {
        assert_eq!(reply.status, 302);
        assert!(session_cookie(&reply).is_none());
        reply.header("location").unwrap().to_owned()
    };
    let key_error = "/#oidc_error=key";
    let log = Audit::new(dir.path().join("audit.log"));
    let last_error = || {
        let events = log.read(1, None).unwrap();
        events[0].detail["error"].as_str().unwrap_or("").to_owned()
    };
    type Change = Box<dyn FnOnce(&mut Idp, &mut String)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "another sign-in",
            Box::new(|i, _| i.next = json!({"nonce": "x"})),
        ),
        (
            "another client",
            Box::new(|i, _| i.next = json!({"aud": "other"})),
        ),
        (
            "another client",
            Box::new(|i, _| i.next = json!({"aud": [CLIENT, "other"]})),
        ),
        (
            "another issuer",
            Box::new(|i, _| i.next = json!({"iss": "https://evil"})),
        ),
        (
            "expired",
            Box::new(|i, _| i.next = json!({"exp": jiff::Timestamp::now().as_second() - 120})),
        ),
        ("expired", Box::new(|i, _| i.next = json!({"exp": null}))),
        (
            "not verified",
            Box::new(|i, _| i.next = json!({"email_verified": false})),
        ),
        (
            "signature is wrong",
            Box::new(|i, _| i.forge = Some(new_key())),
        ),
        (
            "none are not accepted",
            Box::new(|i, _| i.alg = "none".to_owned()),
        ),
        (
            "HS256 are not accepted",
            Box::new(|i, _| i.alg = "HS256".to_owned()),
        ),
        (
            "no user of this server",
            Box::new(|i, _| i.next = json!({"email": "okan@example.com"})),
        ),
        ("another browser", Box::new(|_, s| s.push('x'))),
    ];
    for (why, change) in cases {
        assert_eq!(refused(sign_in(address, &idp, change)), key_error, "{why}");
        assert!(last_error().contains(why), "{why}: {}", last_error());
    }

    // A code can be used once; a state comes back once.
    let started = send(address, "GET", "/api/oidc/start", "", "");
    let asked = query(started.header("location").unwrap());
    let cookie = started.cookies()[0].to_owned();
    idp.lock().unwrap().codes.insert(
        "c1".to_owned(),
        (asked["nonce"].clone(), asked["code_challenge"].clone()),
    );
    let path = format!("/api/oidc/callback?code=c1&state={}", asked["state"]);
    assert!(session_cookie(&send(address, "GET", &path, &cookie, "")).is_some());
    assert_eq!(refused(send(address, "GET", &path, &cookie, "")), key_error);

    // An error from the provider comes back to the sign-in screen.
    let reply = send(
        address,
        "GET",
        "/api/oidc/callback?error=access_denied",
        "",
        "",
    );
    assert_eq!(refused(reply), key_error);

    // A changed identity ends the user's sessions.
    let mut users = Users::load(&file).unwrap();
    users.get_mut("ayse").unwrap().oidc = Some("ayse@other.example".to_owned());
    users.save(&file).unwrap();
    let session = send(address, "GET", "/api/session", &cookie_of(&done), "");
    assert_eq!(
        serde_json::from_str::<Value>(&session.body).unwrap()["signed_in"],
        false
    );

    // A sign-in that cannot even start is recorded too.
    let mut users = Users::load(&file).unwrap();
    users.oidc = None;
    users.save(&file).unwrap();
    let started = send(address, "GET", "/api/oidc/start", "", "");
    assert_eq!(started.header("location"), Some("/#oidc_error=unsupported"));
    assert!(
        last_error().contains("no identity provider"),
        "{}",
        last_error()
    );

    // Each sign-in is recorded with the identity and how it went.
    let events = log.read(1000, None).unwrap();
    let ok = events
        .iter()
        .filter(|e| e.event == "sign_in" && e.outcome == "ok" && e.detail["via"] == "oidc")
        .count();
    assert_eq!(ok, 3);
    assert!(events.iter().any(|e| e.event == "sign_in"
        && e.outcome == "key"
        && e.detail["identity"] == "okan@example.com"));
    log.verify().unwrap();
}

fn cookie_of(reply: &Reply) -> String {
    session_cookie(reply).unwrap()
}

#[test]
fn settings_that_cannot_work_are_refused() {
    let config = |issuer: &str, address: &str| OidcConfig {
        issuer: issuer.to_owned(),
        client_id: CLIENT.to_owned(),
        client_secret: None,
        address: address.to_owned(),
        claim: "email".to_owned(),
    };
    let ok = "https://login.example.com";
    assert!(config(ok, "https://backup.example.com").check().is_ok());
    assert!(
        config("http://127.0.0.1:9000", "http://localhost:8443")
            .check()
            .is_ok()
    );
    for (issuer, address) in [
        ("http://login.example.com", "https://backup.example.com"),
        (ok, "http://backup.example.com"),
        ("login.example.com", "https://backup.example.com"),
    ] {
        assert!(
            config(issuer, address).check().is_err(),
            "{issuer} {address}"
        );
    }

    // Two users cannot have the same identity.
    let user = |name: &str, id: &str| User {
        name: name.to_owned(),
        password: String::new(),
        role: Role::Admin,
        backups: Vec::new(),
        folders: Vec::new(),
        oidc: Some(id.to_owned()),
    };
    let users = Users {
        oidc: Some(config(ok, "https://backup.example.com")),
        users: vec![user("a", "x@example.com"), user("b", "X@example.com")],
    };
    let dir = tempfile::tempdir().unwrap();
    assert!(users.save(&dir.path().join("web.json")).is_err());
}
