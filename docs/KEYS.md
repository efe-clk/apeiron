> **Türkçe:** [KEYS.tr.md](KEYS.tr.md)

# Key Management

> **Status: Done.** An encrypted repository (FORMAT.md §14) opens with a password, a recovery key, or now a key held by a key management service: HashiCorp Vault, AWS KMS, Google Cloud KMS or a hardware security module through PKCS#11. A recovery key can be split so that several people must come together to use it. The decision behind it is ADR-060 ([DECISIONS.md](DECISIONS.md)).

## 1. Commands

| Command | What it does |
| --- | --- |
| `apeiron key add --external <provider>:<key>` | Adds a key held by a service (§2); it is tried once before it is added |
| `apeiron key rotate <id> [--to <provider>:<key>]` | Replaces an external key with a new one, with a new key-encryption key, held by the same service, or by another with `--to`, then removes the old one |
| `apeiron key split [--shares <n>] [--threshold <k>]` | Adds a new recovery key and shows it only as `n` shares (5 by default) of which any `k` (3) give it back (§3) |
| `apeiron key join <share>...` | Puts a recovery key together from its shares and shows it |
| `apeiron key list` | Shows each key with its kind; an external one with its service and key |

A command that opens an encrypted repository without `--password-file` first asks the services of its external keys, in order, and asks for a password only when none of them opens it. A job of the background service without `password_file` does the same, so a server can open its repository without a password on its disk.

## 2. Keys held by a service

The repository's own keys (`id_key` and `data_key`) never leave the computer. An external key file seals them with a key-encryption key of 32 random bytes, and the service keeps only that key-encryption key, wrapped with its own key (FORMAT.md §14.2). Opening asks the service to unwrap it, with the credentials of whoever opens the repository, so taking a key away at the service, or its permission, closes the repository for them.

`apeiron key rotate` makes a new key-encryption key and a new key file and removes the old one; the data is not encrypted again, since `data_key` does not change. Rotating the service's own key, as Vault's `rotate` and AWS's yearly rotation do, needs nothing from Apeiron: they keep unwrapping older versions.

### 2.1 HashiCorp Vault

`vault:<name>` or `vault:<mount>/<name>`: a key of the transit secrets engine, at `transit/` by default. The address and token come from `VAULT_ADDR` and `VAULT_TOKEN`, and `VAULT_NAMESPACE` when set, as for Vault's own command. Tried with Vault 1.20 in dev mode; CI tries it on every push.

### 2.2 AWS KMS

`aws-kms:<key id, ARN or alias>`. The credentials come from `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`, the region from the ARN or `AWS_REGION`; `AWS_ENDPOINT_URL_KMS` or `AWS_ENDPOINT_URL` names another endpoint. Profiles, single sign-on and instance roles are not read: their credentials are given through the variables, for example with `aws configure export-credentials --format env`. Tried with LocalStack 4.9; CI tries it on every push.

### 2.3 Google Cloud KMS

`gcp-kms:projects/<p>/locations/<l>/keyRings/<r>/cryptoKeys/<k>`. The access token comes from `GOOGLE_OAUTH_ACCESS_TOKEN`, or else from `gcloud auth print-access-token`. Not tried yet: there is no emulator of Cloud KMS.

### 2.4 PKCS#11

`pkcs11:module=<library>,token=<label>,key=<label>`: an AES key on a hardware security module, found by the token's and the key's labels; the user PIN comes from `APEIRON_PKCS11_PIN`. The key-encryption key is encrypted with AES-GCM inside the module, so the AES key never leaves it. Tried with SoftHSM 2.7.

## 3. Splitting a recovery key

A recovery key opens the repository alone, so whoever keeps it can open it. `apeiron key split` adds a new recovery key and shows it only as shares of Shamir's secret sharing: any `k` of the `n` shares give the key back with `apeiron key join`, and fewer tell nothing about it. The shares are not stored; each is shown once, to be given to a different person.

A share reads `<k>-<x>-<hex>-<check>`; the check is part of a BLAKE3 hash of the rest, so a mistyped share is refused instead of giving a wrong key. The shares come from the 32 bytes of the recovery key, in GF(256) with a random polynomial per byte.

## 4. Not part of this work item

| Item | Where |
| --- | --- |
| Encrypting the data again with a new `data_key` | Not planned: it would rewrite every pack; copy the repository into a new one instead |
| AWS profiles, single sign-on and instance roles | Later; their credentials go through the environment meanwhile |
| Google Cloud service account files | Later; `gcloud` gives a token meanwhile |
| Trying Google Cloud KMS | With its first user |

## 5. Crates

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-core` | External key files: writing, listing and opening with an unwrapped key | Done |
| `apeiron-kms` | The services, and Shamir's secret sharing | Done |
| `apeiron-cli`, `apeiron-daemon` | The commands, and opening with external keys | Done |
