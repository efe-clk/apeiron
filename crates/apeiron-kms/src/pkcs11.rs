//! A hardware security module through PKCS#11 (KEYS.md §2.4). The key is
//! `module=<library>,token=<token label>,key=<AES key label>`; the user PIN
//! comes from `APEIRON_PKCS11_PIN`. The key-encryption key is encrypted with
//! AES-GCM inside the module, so the AES key never leaves it.

use cryptoki::context::{CInitializeArgs, CInitializeFlags, Pkcs11};
use cryptoki::mechanism::Mechanism;
use cryptoki::mechanism::aead::GcmParams;
use cryptoki::object::{Attribute, ObjectClass, ObjectHandle};
use cryptoki::session::{Session, UserType};
use cryptoki::types::AuthPin;

use crate::{Error, Result, env, service};

const NAME: &str = "pkcs11";

struct Spec {
    module: String,
    token: String,
    key: String,
}

fn spec(key: &str) -> Result<Spec> {
    let mut module = None;
    let mut token = None;
    let mut label = None;
    for part in key.split(',') {
        match part.split_once('=') {
            Some(("module", v)) => module = Some(v.to_owned()),
            Some(("token", v)) => token = Some(v.to_owned()),
            Some(("key", v)) => label = Some(v.to_owned()),
            _ => {}
        }
    }
    match (module, token, label) {
        (Some(module), Some(token), Some(key)) => Ok(Spec { module, token, key }),
        _ => Err(Error::Invalid(
            "a pkcs11 key is module=<library>,token=<label>,key=<label>".to_owned(),
        )),
    }
}

fn session(spec: &Spec) -> Result<(Pkcs11, Session, ObjectHandle)> {
    let fail = |e: cryptoki::error::Error| service(NAME, e);
    let pkcs11 = Pkcs11::new(&spec.module).map_err(fail)?;
    // A module stays initialized for the whole program; a second key
    // used in the same run finds it so.
    match pkcs11.initialize(CInitializeArgs::new(CInitializeFlags::OS_LOCKING_OK)) {
        Ok(())
        | Err(cryptoki::error::Error::Pkcs11(
            cryptoki::error::RvError::CryptokiAlreadyInitialized,
            _,
        )) => {}
        Err(e) => return Err(fail(e)),
    }
    let mut slot = None;
    for s in pkcs11.get_slots_with_token().map_err(fail)? {
        if pkcs11.get_token_info(s).map_err(fail)?.label() == spec.token {
            slot = Some(s);
        }
    }
    let slot = slot.ok_or_else(|| service(NAME, format!("no token labelled {:?}", spec.token)))?;
    let session = pkcs11.open_ro_session(slot).map_err(fail)?;
    let pin =
        env("APEIRON_PKCS11_PIN").ok_or_else(|| service(NAME, "APEIRON_PKCS11_PIN is not set"))?;
    session
        .login(UserType::User, Some(&AuthPin::from(pin)))
        .map_err(fail)?;
    let found = session
        .find_objects(&[
            Attribute::Class(ObjectClass::SECRET_KEY),
            Attribute::Label(spec.key.as_bytes().to_vec()),
        ])
        .map_err(fail)?;
    let key = *found
        .first()
        .ok_or_else(|| service(NAME, format!("no secret key labelled {:?}", spec.key)))?;
    Ok((pkcs11, session, key))
}

pub(crate) fn wrap(key: &str, kek: &[u8; 32]) -> Result<String> {
    let spec = spec(key)?;
    let (_pkcs11, session, handle) = session(&spec)?;
    let mut iv = [0u8; 12];
    getrandom::fill(&mut iv).map_err(|e| service(NAME, e))?;
    let mut iv_param = iv;
    let params = GcmParams::new(&mut iv_param, &[], 128.into()).map_err(|e| service(NAME, e))?;
    let sealed = session
        .encrypt(&Mechanism::AesGcm(params), handle, kek)
        .map_err(|e| service(NAME, e))?;
    Ok(data_encoding::HEXLOWER.encode(&[iv.as_slice(), &sealed].concat()))
}

pub(crate) fn unwrap(key: &str, wrapped: &str) -> Result<Vec<u8>> {
    let spec = spec(key)?;
    let bytes = data_encoding::HEXLOWER
        .decode(wrapped.as_bytes())
        .map_err(|e| service(NAME, format!("not hex: {e}")))?;
    if bytes.len() < 12 {
        return Err(service(NAME, "too short"));
    }
    let (iv, sealed) = bytes.split_at(12);
    let (_pkcs11, session, handle) = session(&spec)?;
    let mut iv = <[u8; 12]>::try_from(iv).map_err(|_| service(NAME, "bad nonce"))?;
    let params = GcmParams::new(&mut iv, &[], 128.into()).map_err(|e| service(NAME, e))?;
    session
        .decrypt(&Mechanism::AesGcm(params), handle, sealed)
        .map_err(|e| service(NAME, e))
}
