//! Certificates from an ACME service (WEB.md §2.1). What needs a service runs
//! only when `APEIRON_ACME_TEST_URL` names one, such as Pebble.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use apeiron::Repository;
use apeiron_server::{Acme, Audit, Role, Shares, Tls, User, Users, Web, WebOptions};

fn start(dir: &Path, listen: SocketAddr, acme: Acme) -> apeiron::Result<Web> {
    let users = dir.join("web.json");
    if !users.exists() {
        Users {
            oidc: None,
            users: vec![User {
                name: "efe".to_owned(),
                password: apeiron_server::hash_password("correct horse").unwrap(),
                role: Role::Admin,
                backups: Vec::new(),
                folders: Vec::new(),
                oidc: None,
                totp: None,
                from_groups: false,
            }],
        }
        .save(&users)
        .unwrap();
        fs::create_dir_all(dir.join("home")).unwrap();
        Repository::init(dir.join("repo")).unwrap();
    }
    Web::start(WebOptions {
        repo: Repository::open(dir.join("repo")).unwrap(),
        listen,
        tls: Some(Tls::acme(acme)),
        users,
        folders: vec![dir.join("home")],
        audit: Audit::new(dir.join("audit.log")),
        shares: Shares::new(dir.join("shares.json")),
        replicas: Vec::new(),
        standby: false,
    })
}

fn acme(dir: &Path) -> Acme {
    Acme {
        domains: vec!["backup.example.com".to_owned()],
        contact: "admin@example.com".to_owned(),
        // Nothing listens here, so no certificate ever comes.
        directory: "https://127.0.0.1:9/directory".to_owned(),
        ca_pem: None,
        cache: dir.join("acme"),
    }
}

#[test]
fn settings_that_cannot_work_stop_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let listen = "127.0.0.1:0".parse().unwrap();
    let mut none = acme(dir.path());
    none.domains.clear();
    assert!(start(dir.path(), listen, none).is_err());
    let mut contact = acme(dir.path());
    contact.contact = "not an address".to_owned();
    assert!(start(dir.path(), listen, contact).is_err());
    let mut ca = acme(dir.path());
    ca.ca_pem =
        Some(b"-----BEGIN CERTIFICATE-----\nnonsense\n-----END CERTIFICATE-----\n".to_vec());
    assert!(start(dir.path(), listen, ca).is_err());
}

#[test]
fn the_server_runs_without_a_certificate_and_keeps_its_folder_private() {
    let dir = tempfile::tempdir().unwrap();
    let server = start(dir.path(), "127.0.0.1:0".parse().unwrap(), acme(dir.path())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join("acme"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    // No certificate yet: the handshake ends, and the server goes on.
    for _ in 0..2 {
        let mut tcp = TcpStream::connect(server.address()).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let name = "backup.example.com".try_into().unwrap();
        let mut conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
        let mut stream = rustls::Stream::new(&mut conn, &mut tcp);
        assert!(stream.write_all(b"GET / HTTP/1.1\r\n\r\n").is_err());
    }
}

/// Connects to the server under `name`, trusting only `roots`, and asks for
/// the health of the server.
fn health(address: SocketAddr, name: &str, roots: &[u8]) -> Result<String, String> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;

    let mut store = rustls::RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(roots) {
        store.add(certificate.unwrap()).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    let server = name.to_owned().try_into().unwrap();
    let mut conn = rustls::ClientConnection::new(Arc::new(config), server).unwrap();
    let mut tcp = TcpStream::connect(address).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut stream = rustls::Stream::new(&mut conn, &mut tcp);
    let request = format!("GET /api/health HTTP/1.1\r\nHost: {name}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    Ok(String::from_utf8_lossy(&reply).into_owned())
}

/// A certificate comes from the service, a browser that trusts the service
/// accepts it, and a restart uses the one kept instead of asking again.
#[test]
fn a_certificate_comes_from_the_service() {
    let Ok(directory) = std::env::var("APEIRON_ACME_TEST_URL") else {
        eprintln!("skipped: APEIRON_ACME_TEST_URL is not set");
        return;
    };
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let ca_pem = fs::read(var("APEIRON_ACME_TEST_CA")).unwrap();
    let roots = fs::read(var("APEIRON_ACME_TEST_ROOT")).unwrap();
    let listen: SocketAddr = var("APEIRON_ACME_TEST_LISTEN").parse().unwrap();
    let name = var("APEIRON_ACME_TEST_DOMAIN");

    let dir = tempfile::tempdir().unwrap();
    let settings = Acme {
        domains: vec![name.clone()],
        contact: "admin@example.com".to_owned(),
        directory,
        ca_pem: Some(ca_pem),
        cache: dir.path().join("acme"),
    };
    let server = start(dir.path(), listen, settings.clone()).unwrap();
    let start_time = Instant::now();
    let reply = loop {
        match health(server.address(), &name, &roots) {
            Ok(reply) => break reply,
            Err(e) if start_time.elapsed() > Duration::from_secs(120) => {
                panic!("no certificate in two minutes: {e}")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    };
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    drop(server);

    let kept = fs::read_dir(dir.path().join("acme")).unwrap().count();
    assert!(kept >= 2, "the certificate and the account key are kept");
    let again = start(dir.path(), listen, settings).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let reply = health(again.address(), &name, &roots).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
}
