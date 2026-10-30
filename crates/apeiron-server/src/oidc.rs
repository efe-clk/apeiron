//! Signing in through an organization's identity provider with OpenID
//! Connect (IDENTITY.md): the authorization code flow with PKCE, and the ID
//! token checked against the provider's published keys.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use apeiron::{Error, ErrorKind};
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents, UnparsedPublicKey,
};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{invalid, lock};

/// How long a sign-in may take at the provider (IDENTITY.md §3).
const PENDING: Duration = Duration::from_secs(10 * 60);
/// Sign-ins started and not finished, at most.
const MAX_PENDING: usize = 1000;
/// How long the provider's settings are kept before they are read again.
const DISCOVERED: Duration = Duration::from_secs(60 * 60);
/// The keys are read again for an unknown key at most this often.
const KEYS_AGAIN: Duration = Duration::from_secs(60);
/// Clocks of the server and the provider may differ by this much.
const SKEW: i64 = 60;

/// The identity provider, as kept in the users file (IDENTITY.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidcConfig {
    /// The provider's issuer URL, such as `https://login.example.com/realms/corp`.
    pub issuer: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// The address browsers reach this server at, such as
    /// `https://backup.example.com`.
    pub address: String,
    /// The claim of the ID token that names the user.
    #[serde(default = "default_claim")]
    pub claim: String,
}

fn default_claim() -> String {
    "email".to_owned()
}

/// `https`, or `http` to this computer only, for tests and a provider on
/// the same machine.
fn checked_url(what: &str, text: &str) -> Result<Url, Error> {
    let url = Url::parse(text).map_err(|e| invalid(format!("{what} {text}: {e}")))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(invalid(format!("{what} {text} must start with https://"))),
    }
}

impl OidcConfig {
    /// Fails unless the settings can work.
    pub fn check(&self) -> Result<(), Error> {
        checked_url("issuer", &self.issuer)?;
        checked_url("address", &self.address)?;
        if self.client_id.is_empty() || self.claim.is_empty() {
            return Err(invalid("the client id and the claim must not be empty"));
        }
        Ok(())
    }

    /// Where the provider sends the browser back to.
    pub fn redirect(&self) -> String {
        format!("{}/api/oidc/callback", self.address.trim_end_matches('/'))
    }

    /// Whether `value`, from the ID token, is `known`, a user's `oidc`;
    /// e-mail addresses are compared without case.
    pub fn same(&self, known: &str, value: &str) -> bool {
        if self.claim == "email" {
            known.eq_ignore_ascii_case(value)
        } else {
            known == value
        }
    }
}

/// What the provider publishes about itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Provider {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

/// One public key of the provider (RFC 7517).
#[derive(Clone, Debug, Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default, rename = "use")]
    use_: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// A sign-in started and not finished.
struct Pending {
    nonce: String,
    verifier: String,
    made: Instant,
}

fn refused(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Key, message)
}

fn provider_error(message: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorKind::Io,
        format!("the identity provider cannot be reached: {message}"),
    )
}

fn random(bytes: usize) -> Result<String, Error> {
    let mut raw = vec![0u8; bytes];
    getrandom::fill(&mut raw).map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

fn decode(part: &str) -> Result<Vec<u8>, Error> {
    URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('='))
        .map_err(|_| refused("the ID token is not well formed"))
}

/// Sign-ins through the provider in the users file.
pub(crate) struct Oidc {
    http: reqwest::Client,
    discovered: tokio::sync::Mutex<Option<(String, Provider, Instant)>>,
    keys: tokio::sync::Mutex<Option<(String, Jwks, Instant)>>,
    pending: Mutex<HashMap<String, Pending>>,
}

impl Oidc {
    pub(crate) fn new() -> Result<Oidc, Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(provider_error)?;
        Ok(Oidc {
            http,
            discovered: tokio::sync::Mutex::new(None),
            keys: tokio::sync::Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, Error> {
        let response = self.http.get(url).send().await.map_err(provider_error)?;
        if !response.status().is_success() {
            return Err(provider_error(format!("{url}: {}", response.status())));
        }
        let body = response.bytes().await.map_err(provider_error)?;
        serde_json::from_slice(&body).map_err(|e| provider_error(format!("{url}: {e}")))
    }

    /// The provider's settings, read again after an hour.
    pub(crate) async fn provider(&self, config: &OidcConfig) -> Result<Provider, Error> {
        let mut kept = self.discovered.lock().await;
        if let Some((issuer, provider, at)) = &*kept
            && *issuer == config.issuer
            && at.elapsed() < DISCOVERED
        {
            return Ok(provider.clone());
        }
        let provider = discover(&self.http, config).await?;
        *kept = Some((config.issuer.clone(), provider.clone(), Instant::now()));
        Ok(provider)
    }

    /// Where to send the browser to sign in, and the state that comes back.
    pub(crate) async fn start(&self, config: &OidcConfig) -> Result<(String, String), Error> {
        let provider = self.provider(config).await?;
        let (state, nonce, verifier) = (random(32)?, random(32)?, random(32)?);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = checked_url("authorization endpoint", &provider.authorization_endpoint)?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &config.redirect())
            .append_pair("scope", "openid email profile")
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        let mut pending = lock(&self.pending);
        pending.retain(|_, p| p.made.elapsed() < PENDING);
        if pending.len() >= MAX_PENDING {
            return Err(Error::new(
                ErrorKind::Locked,
                "too many sign-ins at once; try again in a minute",
            ));
        }
        pending.insert(
            state.clone(),
            Pending {
                nonce,
                verifier,
                made: Instant::now(),
            },
        );
        Ok((url.to_string(), state))
    }

    /// Finishes a sign-in: trades `code` for an ID token, checks it and
    /// returns the value of the configured claim (IDENTITY.md §3).
    pub(crate) async fn finish(
        &self,
        config: &OidcConfig,
        code: &str,
        state: &str,
    ) -> Result<String, Error> {
        let pending = lock(&self.pending)
            .remove(state)
            .filter(|p| p.made.elapsed() < PENDING)
            .ok_or_else(|| refused("this sign-in has expired; start again"))?;
        let provider = self.provider(config).await?;
        let token = self.token(config, &provider, code, &pending).await?;
        let claims = self
            .verify(config, &provider, &token, &pending.nonce)
            .await?;
        let value = claims
            .get(&config.claim)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| refused(format!("the ID token has no {}", config.claim)))?;
        if config.claim == "email" && claims.get("email_verified") == Some(&Value::Bool(false)) {
            return Err(refused(
                "the identity provider has not verified this e-mail address",
            ));
        }
        Ok(value.to_owned())
    }

    async fn token(
        &self,
        config: &OidcConfig,
        provider: &Provider,
        code: &str,
        pending: &Pending,
    ) -> Result<String, Error> {
        let url = checked_url("token endpoint", &provider.token_endpoint)?;
        let redirect = config.redirect();
        let mut form = vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code.to_owned()),
            ("redirect_uri", redirect),
            ("code_verifier", pending.verifier.clone()),
        ];
        let mut request = self.http.post(url);
        let methods = &provider.token_endpoint_auth_methods_supported;
        match &config.client_secret {
            // The default of the standard, unless the provider names others.
            Some(secret)
                if methods.is_empty() || methods.iter().any(|m| m == "client_secret_basic") =>
            {
                let encode =
                    |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
                let pair = format!("{}:{}", encode(&config.client_id), encode(secret));
                request = request.header(
                    reqwest::header::AUTHORIZATION,
                    format!("Basic {}", STANDARD.encode(pair)),
                );
            }
            Some(secret) => {
                form.push(("client_id", config.client_id.clone()));
                form.push(("client_secret", secret.clone()));
            }
            None => form.push(("client_id", config.client_id.clone())),
        }
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&form)
            .finish();
        let response = request
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(provider_error)?;
        let status = response.status();
        let body: Value = response
            .bytes()
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        if !status.is_success() {
            let reason = body["error"].as_str().unwrap_or("no reason given");
            return Err(refused(format!(
                "the identity provider refused the sign-in: {reason}"
            )));
        }
        body["id_token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| refused("the identity provider sent no ID token"))
    }

    /// The keys of the provider; read again when `kid` is not among them.
    async fn keys(&self, provider: &Provider, kid: Option<&str>) -> Result<Jwks, Error> {
        let mut kept = self.keys.lock().await;
        if let Some((uri, jwks, at)) = &*kept
            && *uri == provider.jwks_uri
        {
            let known = kid.is_none_or(|k| jwks.keys.iter().any(|j| j.kid.as_deref() == Some(k)));
            if known || at.elapsed() < KEYS_AGAIN {
                return Ok(jwks.clone());
            }
        }
        checked_url("key set", &provider.jwks_uri)?;
        let jwks: Jwks = self.get_json(&provider.jwks_uri).await?;
        *kept = Some((provider.jwks_uri.clone(), jwks.clone(), Instant::now()));
        Ok(jwks)
    }

    async fn verify(
        &self,
        config: &OidcConfig,
        provider: &Provider,
        token: &str,
        nonce: &str,
    ) -> Result<Map<String, Value>, Error> {
        let parts: Vec<&str> = token.split('.').collect();
        let [header, payload, signature] = parts[..] else {
            return Err(refused("the ID token is not well formed"));
        };
        let header: Value = serde_json::from_slice(&decode(header)?)
            .map_err(|_| refused("the ID token is not well formed"))?;
        let alg = header["alg"].as_str().unwrap_or("");
        let kid = header["kid"].as_str();
        let keys = self.keys(provider, kid).await?;
        let signed = format!("{}.{}", parts[0], parts[1]);
        check_signature(&keys, alg, kid, signed.as_bytes(), &decode(signature)?)?;
        let claims: Map<String, Value> = serde_json::from_slice(&decode(payload)?)
            .map_err(|_| refused("the ID token is not well formed"))?;
        check_claims(
            config,
            provider,
            &claims,
            nonce,
            jiff::Timestamp::now().as_second(),
        )?;
        Ok(claims)
    }
}

/// Reads the provider's settings from its well-known address.
async fn discover(http: &reqwest::Client, config: &OidcConfig) -> Result<Provider, Error> {
    let base = checked_url("issuer", &config.issuer)?;
    let url = format!(
        "{}/.well-known/openid-configuration",
        base.as_str().trim_end_matches('/')
    );
    let response = http.get(&url).send().await.map_err(provider_error)?;
    if !response.status().is_success() {
        return Err(provider_error(format!("{url}: {}", response.status())));
    }
    let body = response.bytes().await.map_err(provider_error)?;
    let provider: Provider =
        serde_json::from_slice(&body).map_err(|e| provider_error(format!("{url}: {e}")))?;
    if provider.issuer != config.issuer {
        return Err(provider_error(format!(
            "it calls itself {}, not {}",
            provider.issuer, config.issuer
        )));
    }
    Ok(provider)
}

/// Reads the provider's settings and keys once, for `apeiron web oidc
/// check`; blocks until done.
pub fn check_provider(config: &OidcConfig) -> Result<(Provider, usize), Error> {
    config.check()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    runtime.block_on(async {
        let oidc = Oidc::new()?;
        let provider = oidc.provider(config).await?;
        let keys = oidc.keys(&provider, None).await?;
        Ok((provider, keys.keys.len()))
    })
}

/// Checks the token's signature with a key of the provider; only RS256
/// and ES256 are accepted, never `none` or a shared secret.
fn check_signature(
    keys: &Jwks,
    alg: &str,
    kid: Option<&str>,
    signed: &[u8],
    signature: &[u8],
) -> Result<(), Error> {
    let kty = match alg {
        "RS256" => "RSA",
        "ES256" => "EC",
        _ => {
            return Err(refused(format!(
                "ID tokens signed with {alg} are not accepted"
            )));
        }
    };
    let fits = |k: &&Jwk| {
        k.kty == kty
            && k.use_.as_deref().is_none_or(|u| u == "sig")
            && kid.is_none_or(|kid| k.kid.as_deref() == Some(kid))
    };
    let bad = || refused("the ID token's signature is wrong");
    for key in keys.keys.iter().filter(fits) {
        let ok = match alg {
            "RS256" => {
                let (Some(n), Some(e)) = (&key.n, &key.e) else {
                    continue;
                };
                RsaPublicKeyComponents {
                    n: decode(n)?,
                    e: decode(e)?,
                }
                .verify(&RSA_PKCS1_2048_8192_SHA256, signed, signature)
                .is_ok()
            }
            _ => {
                let (Some("P-256"), Some(x), Some(y)) = (key.crv.as_deref(), &key.x, &key.y) else {
                    continue;
                };
                let (x, y) = (decode(x)?, decode(y)?);
                if x.len() != 32 || y.len() != 32 {
                    continue;
                }
                let point = [&[4u8][..], &x, &y].concat();
                UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
                    .verify(signed, signature)
                    .is_ok()
            }
        };
        if ok {
            return Ok(());
        }
    }
    Err(bad())
}

/// Checks who issued the token, for whom, until when, and for which
/// sign-in (OpenID Connect Core §3.1.3.7).
fn check_claims(
    config: &OidcConfig,
    provider: &Provider,
    claims: &Map<String, Value>,
    nonce: &str,
    now: i64,
) -> Result<(), Error> {
    if claims.get("iss").and_then(Value::as_str) != Some(provider.issuer.as_str()) {
        return Err(refused("the ID token comes from another issuer"));
    }
    let client = config.client_id.as_str();
    let audience: Vec<&str> = match claims.get("aud") {
        Some(Value::String(a)) => vec![a.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    if !audience.contains(&client) {
        return Err(refused("the ID token is meant for another client"));
    }
    if audience.len() > 1 && claims.get("azp").and_then(Value::as_str) != Some(client) {
        return Err(refused("the ID token is meant for another client"));
    }
    match claims.get("exp").and_then(Value::as_i64) {
        Some(exp) if now < exp + SKEW => {}
        _ => return Err(refused("the ID token has expired")),
    }
    if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Err(refused("the ID token belongs to another sign-in"));
    }
    Ok(())
}
