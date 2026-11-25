//! Wrapping and unwrapping with real services, when CI or a developer
//! starts them: Vault in dev mode (`VAULT_ADDR`, `VAULT_TOKEN` and the
//! transit key `APEIRON_TEST_VAULT_KEY`) and AWS KMS of LocalStack
//! (`AWS_ENDPOINT_URL` and `APEIRON_TEST_AWS_KEY`). Skipped otherwise.

fn round_trip(provider: &str, key: &str) {
    let kek = apeiron_kms::new_kek().unwrap();
    let wrapped = apeiron_kms::wrap(provider, key, &kek).unwrap();
    assert!(!wrapped.is_empty());
    assert_eq!(apeiron_kms::unwrap(provider, key, &wrapped).unwrap(), kek);
    assert!(apeiron_kms::unwrap(provider, key, "not wrapped").is_err());
}

#[test]
fn vault_wraps_and_unwraps() {
    match std::env::var("APEIRON_TEST_VAULT_KEY") {
        Ok(key) => round_trip("vault", &key),
        Err(_) => eprintln!("skipped: APEIRON_TEST_VAULT_KEY is not set"),
    }
}

#[test]
fn aws_kms_wraps_and_unwraps() {
    match std::env::var("APEIRON_TEST_AWS_KEY") {
        Ok(key) => round_trip("aws-kms", &key),
        Err(_) => eprintln!("skipped: APEIRON_TEST_AWS_KEY is not set"),
    }
}
