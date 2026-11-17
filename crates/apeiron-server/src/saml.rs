//! Signing in through an identity provider with SAML 2.0 (IDENTITY.md §6):
//! this server is a service provider; the provider's signed response is
//! checked by `saml-rs`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use apeiron::{Error, ErrorKind};
use saml_rs::{
    AcsEndpoint, BrowserInput, EntityId, FormField, IdpDescriptor, InMemoryReplayCache,
    MetadataTrustPolicy, PendingAuthnRequest, RelayStateParam, ReplayPolicy, SamlValidationContext,
    SpConfig, SsoResponse, StartSso,
};
use serde::{Deserialize, Serialize};

use crate::oidc::GroupRule;
use crate::{invalid, lock};

/// How long a sign-in may take at the provider.
const PENDING: Duration = Duration::from_secs(10 * 60);
/// Sign-ins started and not finished that are kept at most.
const MAX_PENDING: usize = 10_000;

/// The SAML settings in the users file (IDENTITY.md §6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlConfig {
    /// The provider's metadata, as XML, from a file the admin gave.
    pub idp_metadata: String,
    /// The provider's entity id, read from its metadata.
    pub idp_entity_id: String,
    /// The address browsers reach this server at, such as
    /// `https://backup.example.com`.
    pub address: String,
    /// The attribute that names the user; the subject's `NameID` when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    /// The attribute that lists the user's groups (IDENTITY.md §2.1).
    #[serde(default = "default_groups")]
    pub groups_attribute: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupRule>,
    /// This server's private key, PKCS #8 PEM, made by [`SamlConfig::new`];
    /// it signs the sign-in requests, which most providers ask for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sp_key: String,
    /// Its self-signed certificate, in this server's metadata.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sp_certificate: String,
}

fn default_groups() -> String {
    "groups".to_owned()
}

fn saml_error(e: saml_rs::SamlError) -> Error {
    Error::new(ErrorKind::Key, format!("SAML: {e}"))
}

impl SamlConfig {
    /// Settings from the provider's metadata, with its entity id read from
    /// the metadata's root element.
    pub fn new(idp_metadata: String, address: String) -> Result<Self, Error> {
        let idp_entity_id =
            entity_id_of(&idp_metadata).ok_or_else(|| invalid("the metadata names no entityID"))?;
        let (sp_key, sp_certificate) = signing_pair(&address)?;
        let config = SamlConfig {
            idp_metadata,
            idp_entity_id,
            address,
            attribute: None,
            groups_attribute: default_groups(),
            groups: Vec::new(),
            sp_key,
            sp_certificate,
        };
        config.check()?;
        Ok(config)
    }

    /// Keeps the key and certificate of `old`, so a provider that has this
    /// server's certificate keeps trusting it.
    pub fn keep_keys(&mut self, old: &SamlConfig) {
        if !old.sp_key.is_empty() {
            self.sp_key.clone_from(&old.sp_key);
            self.sp_certificate.clone_from(&old.sp_certificate);
        }
    }

    /// Fails unless the settings can work.
    pub fn check(&self) -> Result<(), Error> {
        let address = self.address.trim_end_matches('/');
        let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
            .iter()
            .any(|p| address.starts_with(p));
        if !address.starts_with("https://") && !local {
            return Err(invalid(format!(
                "address {address} must start with https://"
            )));
        }
        self.sp()?;
        self.idp()?;
        Ok(())
    }

    /// The entity id of this server: the address of its metadata.
    pub fn entity_id(&self) -> String {
        format!("{}/api/saml/metadata", self.address.trim_end_matches('/'))
    }

    /// Where the provider posts its response.
    pub fn acs(&self) -> String {
        format!("{}/api/saml/acs", self.address.trim_end_matches('/'))
    }

    fn sp(&self) -> Result<saml_rs::Saml<saml_rs::Sp>, Error> {
        let entity = EntityId::try_new(self.entity_id()).map_err(saml_error)?;
        let acs = AcsEndpoint::post(self.acs()).map_err(saml_error)?;
        let mut builder = SpConfig::builder(entity).acs_endpoint(acs);
        if !self.sp_key.is_empty() {
            builder = builder
                .credentials(saml_rs::Credentials {
                    signing_key: Some(saml_rs::PrivateKeyPem::new(self.sp_key.clone())),
                    signing_certificate: Some(saml_rs::CertificatePem::new(
                        self.sp_certificate.clone(),
                    )),
                    ..saml_rs::Credentials::default()
                })
                .validation(saml_rs::SpValidationPolicy {
                    authn_requests: saml_rs::AuthnRequestSigningPolicy::Sign,
                    ..saml_rs::SpValidationPolicy::recommended()
                });
        }
        let config = builder.build().map_err(saml_error)?;
        saml_rs::Saml::sp(config).map_err(saml_error)
    }

    fn idp(&self) -> Result<IdpDescriptor, Error> {
        let entity = EntityId::try_new(self.idp_entity_id.clone()).map_err(saml_error)?;
        // The metadata comes from a file the admin chose, not from the
        // network, so it is trusted as given; the responses it describes
        // must still be signed with its keys.
        IdpDescriptor::from_metadata_xml_for(
            entity,
            &self.idp_metadata,
            MetadataTrustPolicy::UnsignedForCompatibility,
        )
        .map_err(saml_error)
    }

    /// This server's metadata, for the provider.
    pub fn metadata(&self) -> Result<String, Error> {
        Ok(self.sp()?.metadata_xml().to_owned())
    }
}

/// A new RSA key of 2048 bits and a self-signed certificate for it, valid
/// for ten years: the SAML signing pair of this server.
fn signing_pair(address: &str) -> Result<(String, String), Error> {
    use aws_lc_rs::encoding::AsDer;
    let failed =
        |e: rcgen::Error| Error::new(ErrorKind::Internal, format!("cannot make a key: {e}"));
    let rsa = aws_lc_rs::rsa::KeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048)
        .and_then(|k| k.as_der())
        .map_err(|_| Error::new(ErrorKind::Internal, "cannot make an RSA key"))?;
    let der = rustls::pki_types::PrivatePkcs8KeyDer::from(rsa.as_ref().to_vec());
    let key = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_RSA_SHA256)
        .map_err(failed)?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(failed)?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, format!("Apeiron SAML {address}"));
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2035, 1, 1);
    let cert = params.self_signed(&key).map_err(failed)?;
    Ok((key.serialize_pem(), cert.pem()))
}

/// The `entityID` attribute of the first element of `xml`, read plainly; the
/// metadata parser checks it again.
fn entity_id_of(xml: &str) -> Option<String> {
    let start = xml.find("entityID=")? + "entityID=".len();
    let quote = xml[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &xml[start + 1..];
    Some(rest[..rest.find(quote)?].to_owned())
}

/// Who signed in: the value naming the user and the groups.
pub(crate) struct Signed {
    pub(crate) value: String,
    pub(crate) groups: Vec<String>,
}

/// Sign-ins in progress and the assertions already used.
pub(crate) struct Saml {
    pending: Mutex<HashMap<String, (PendingAuthnRequest, Instant)>>,
    replay: Mutex<InMemoryReplayCache>,
}

impl Saml {
    pub(crate) fn new() -> Saml {
        Saml {
            pending: Mutex::new(HashMap::new()),
            replay: Mutex::new(InMemoryReplayCache::new().with_max_entries(100_000)),
        }
    }

    /// Where to send the browser, and the state the browser keeps.
    pub(crate) fn start(&self, config: &SamlConfig, state: String) -> Result<String, Error> {
        let sp = config.sp()?;
        let idp = config.idp()?;
        let relay = RelayStateParam::try_from_option(Some(state.clone())).map_err(saml_error)?;
        let started = sp
            .start_sso(&idp, StartSso::redirect().relay_state(relay))
            .map_err(saml_error)?;
        let url = started
            .outbound
            .redirect_url()
            .map_err(saml_error)?
            .to_owned();
        let mut pending = lock(&self.pending);
        let now = Instant::now();
        pending.retain(|_, (_, made)| now - *made < PENDING);
        if pending.len() >= MAX_PENDING {
            return Err(Error::new(
                ErrorKind::Locked,
                "too many sign-ins in progress; try again in a few minutes",
            ));
        }
        pending.insert(state, (started.pending, now));
        Ok(url)
    }

    /// Checks the provider's response to the sign-in `state` started.
    pub(crate) fn finish(
        &self,
        config: &SamlConfig,
        state: &str,
        form: Vec<(String, String)>,
    ) -> Result<Signed, Error> {
        let pending = lock(&self.pending)
            .remove(state)
            .filter(|(_, made)| made.elapsed() < PENDING)
            .map(|(p, _)| p)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Key,
                    "this sign-in was not started here, or took too long",
                )
            })?;
        let sp = config.sp()?;
        let idp = config.idp()?;
        let fields = form
            .into_iter()
            .map(|(name, value)| FormField::new(name, value))
            .collect();
        let mut replay = lock(&self.replay);
        let validation =
            SamlValidationContext::new(SystemTime::now(), ReplayPolicy::RequireCache(&mut *replay));
        let session = sp
            .finish_sso(
                &idp,
                &pending,
                BrowserInput::<SsoResponse>::post(fields),
                validation,
            )
            .map_err(saml_error)?;
        let values = |name: &str| -> Vec<String> {
            session
                .attributes()
                .get(name)
                .map(|a| a.values().iter().map(|v| v.as_str().to_owned()).collect())
                .unwrap_or_default()
        };
        let value = match &config.attribute {
            Some(name) => values(name).into_iter().next().ok_or_else(|| {
                Error::new(
                    ErrorKind::Key,
                    format!("the provider sent no attribute {name}"),
                )
            })?,
            None => session.name_id().value().to_owned(),
        };
        if value.is_empty() {
            return Err(Error::new(ErrorKind::Key, "the provider named no user"));
        }
        Ok(Signed {
            value,
            groups: values(&config.groups_attribute),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::entity_id_of;

    #[test]
    fn the_entity_id_is_read_from_the_metadata() {
        assert_eq!(
            entity_id_of(r#"<md:EntityDescriptor xmlns:md="x" entityID="https://idp/x">"#),
            Some("https://idp/x".to_owned())
        );
        assert_eq!(
            entity_id_of("<EntityDescriptor entityID='a'>"),
            Some("a".to_owned())
        );
        assert_eq!(entity_id_of("<EntityDescriptor>"), None);
    }
}
