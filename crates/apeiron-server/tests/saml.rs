//! Signing in through a SAML identity provider (IDENTITY.md §6), against a
//! provider made for the test with saml-rs. Its key and certificate in
//! `tests/saml/` are the test-only fixtures of saml-rs 0.8.0 (MIT licence).

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::SystemTime;

use apeiron::Repository;
use apeiron_server::{Audit, Role, SamlConfig, Shares, User, Users, Web, WebOptions};
use saml_rs::{
    AuthnRequest, BrowserInput, CertificatePem, Credentials, EntityId, IdpConfig,
    IdpValidationPolicy, MetadataTrustPolicy, NameId, PrivateKeyPem, ReplayPolicy, RespondSso,
    Saml, SamlValidationContext, SpDescriptor, SsoEndpoint, Subject,
};

const IDP: &str = "https://idp.example.com/metadata";

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

    /// `name=value` of each cookie set.
    fn cookies(&self) -> Vec<String> {
        self.headers
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .filter_map(|(_, v)| v.split(';').next().map(str::to_owned))
            .collect()
    }
}

fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    cookie: &str,
    kind: &str,
    body: &str,
) -> Reply {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nCookie: {cookie}\r\n\
         Content-Type: {kind}\r\nContent-Length: {}\r\n\r\n{body}",
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

fn idp() -> Saml<saml_rs::Idp> {
    let credentials = Credentials {
        signing_key: Some(PrivateKeyPem::new(include_str!("saml/idp-key.pem"))),
        signing_certificate: Some(CertificatePem::new(include_str!("saml/idp-cert.pem"))),
        ..Credentials::default()
    };
    Saml::idp(
        IdpConfig::builder(EntityId::try_new(IDP).unwrap())
            .sso_endpoint(SsoEndpoint::redirect("https://idp.example.com/sso").unwrap())
            .credentials(credentials)
            .validation(IdpValidationPolicy::recommended())
            .build()
            .unwrap(),
    )
    .unwrap()
}

/// The form the provider posts after `ayse` signs in, for the sign-in that
/// sent the browser to `location`.
fn answer(
    idp: &Saml<saml_rs::Idp>,
    sp_metadata: &str,
    location: &str,
    name: &str,
) -> Vec<(String, String)> {
    let sp = SpDescriptor::from_metadata_xml_for(
        EntityId::try_new("https://backup.example.com/api/saml/metadata").unwrap(),
        sp_metadata,
        MetadataTrustPolicy::UnsignedForCompatibility,
    )
    .unwrap();
    let query = location.split_once('?').unwrap().1;
    let request = idp
        .receive_sso(
            &sp,
            BrowserInput::<AuthnRequest>::redirect(query),
            SamlValidationContext::new(SystemTime::now(), ReplayPolicy::DisabledForCompatibility),
        )
        .unwrap();
    let response = idp
        .respond_sso(
            &sp,
            &request,
            Subject::new(NameId::new(name, None), Vec::new()),
            RespondSso::post(),
        )
        .unwrap();
    response
        .post_form()
        .unwrap()
        .fields()
        .iter()
        .map(|f| (f.name().to_owned(), f.value().to_owned()))
        .collect()
}

fn encode(form: &[(String, String)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish()
}

#[test]
fn signing_in_through_a_saml_provider() {
    let dir = tempfile::tempdir().unwrap();
    let idp = idp();
    let file = dir.path().join("config/web.json");
    let mut config = SamlConfig::new(
        idp.metadata_xml().to_owned(),
        "https://backup.example.com".to_owned(),
    )
    .unwrap();
    config.attribute = None;
    let users = Users {
        oidc: None,
        saml: Some(config),
        users: vec![User {
            name: "ayse".to_owned(),
            password: String::new(),
            role: Role::Admin,
            backups: Vec::new(),
            folders: Vec::new(),
            oidc: Some("ayse@example.com".to_owned()),
            totp: None,
            from_groups: false,
            max_bytes: None,
            disabled: false,
        }],
    };
    users.save(&file).unwrap();
    let server = Web::start(WebOptions {
        repo: Repository::init(dir.path().join("repo")).unwrap(),
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: None,
        users: file,
        audit: Audit::new(dir.path().join("audit.log")),
        shares: Shares::new(dir.path().join("shares.json")),
        folders: vec![dir.path().to_path_buf()],
        replicas: Vec::new(),
        standby: false,
        jobs: None,
    })
    .unwrap();
    let a = server.address();
    let json = "application/json";
    let form = "application/x-www-form-urlencoded";

    let session = send(a, "GET", "/api/session", "", json, "");
    assert!(
        session.body.contains("\"provider\":\"saml\""),
        "{}",
        session.body
    );
    let metadata = send(a, "GET", "/api/saml/metadata", "", json, "");
    assert_eq!(metadata.status, 200);
    assert!(
        metadata
            .body
            .contains("https://backup.example.com/api/saml/acs")
    );

    // A sign-in goes to the provider and comes back with a signed answer.
    let started = send(a, "GET", "/api/saml/start", "", json, "");
    assert_eq!(started.status, 302, "{}", started.body);
    let location = started.header("location").unwrap().to_owned();
    assert!(location.starts_with("https://idp.example.com/sso?SAMLRequest="));
    let state = started.cookies()[0].clone();
    assert!(state.starts_with("apeiron_saml="));
    let posted = answer(&idp, &metadata.body, &location, "ayse@example.com");
    let done = send(a, "POST", "/api/saml/acs", &state, form, &encode(&posted));
    assert_eq!(done.status, 302, "{}", done.body);
    assert_eq!(done.header("location"), Some("/"));
    let session_cookie = done
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("apeiron_session="))
        .unwrap();
    let me = send(a, "GET", "/api/session", &session_cookie, json, "");
    assert!(me.body.contains("\"name\":\"ayse\""), "{}", me.body);

    // The same answer again is refused.
    let again = send(a, "POST", "/api/saml/acs", &state, form, &encode(&posted));
    assert_eq!(again.header("location"), Some("/#oidc_error=key"));

    // An answer from another browser, or one changed on the way, is refused.
    let started = send(a, "GET", "/api/saml/start", "", json, "");
    let location = started.header("location").unwrap().to_owned();
    let state = started.cookies()[0].clone();
    let posted = answer(&idp, &metadata.body, &location, "ayse@example.com");
    let other = send(
        a,
        "POST",
        "/api/saml/acs",
        "apeiron_saml=0000",
        form,
        &encode(&posted),
    );
    assert_eq!(other.header("location"), Some("/#oidc_error=key"));
    let changed: Vec<(String, String)> = posted
        .iter()
        .map(|(k, v)| {
            if k != "SAMLResponse" {
                return (k.clone(), v.clone());
            }
            use base64::Engine;
            let engine = base64::engine::general_purpose::STANDARD;
            let xml = String::from_utf8(engine.decode(v).unwrap()).unwrap();
            let forged = xml.replace("ayse@example.com", "root@example.com");
            assert_ne!(xml, forged);
            (k.clone(), engine.encode(forged))
        })
        .collect();
    let forged = send(a, "POST", "/api/saml/acs", &state, form, &encode(&changed));
    assert_eq!(forged.header("location"), Some("/#oidc_error=key"));

    // A person the server does not know is refused.
    let started = send(a, "GET", "/api/saml/start", "", json, "");
    let location = started.header("location").unwrap().to_owned();
    let state = started.cookies()[0].clone();
    let posted = answer(&idp, &metadata.body, &location, "mallory@example.com");
    let unknown = send(a, "POST", "/api/saml/acs", &state, form, &encode(&posted));
    assert_eq!(unknown.header("location"), Some("/#oidc_error=key"));

    let audit = fs::read_to_string(dir.path().join("audit.log")).unwrap();
    assert!(audit.contains("\"via\":\"saml\""), "{audit}");
    assert_eq!(audit.matches("\"sign_in\"").count(), 5, "{audit}");
}
