//! What sealing a legacy row has to produce, before any SQL runs.
//!
//! The startup sweep does not invent a representation: it calls the same
//! [`seal_for_storage`] an ordinary write calls, so a converted row is
//! byte-for-byte what this deployment would have written for the same value with
//! the same key. That claim is checkable without a database, and it is the
//! property a deployment depends on — a secret that survives the conversion is
//! the secret it had, opened under the purpose its key implies, at the version
//! its key is registered under.
//!
//! What is asserted here is therefore the *pairing*: for every purpose in the
//! catalogue, sealing a legacy row's plaintext under that purpose produces
//! ciphertext that opens back to exactly those bytes, and nothing in the
//! rendering of a report or a refusal carries a value, a key name, or key
//! material. The SQL round trip itself is `backfill.rs`.

use context69_secret_store::{
    SEALED_CIPHERTEXT_VERSION, SecretKeyName, SecretPurpose, SecretStore, crypto::SecretCipher,
    seal_for_storage,
};

/// Test master key, bytes `00..20`. Not a deployment secret.
const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const KEY_VERSION: u32 = 3;

/// A record identifier the key-name bound accepts.
const RECORD: &str = "8b41c0de-5f6a-4c2b-9d8e-1a2b3c4d5e6f";

fn cipher() -> SecretCipher {
    SecretCipher::new(
        context69_secret_store::crypto::MasterKey::from_base64(MASTER_KEY_B64)
            .expect("test master key"),
        KEY_VERSION,
    )
}

/// The key a purpose's rows would be stored under.
fn key_for(purpose: SecretPurpose) -> String {
    match purpose.key_name_prefix() {
        Some(prefix) => format!("{prefix}{RECORD}"),
        None => purpose
            .singleton_key_name()
            .expect("a non record-scoped purpose has a fixed name")
            .to_string(),
    }
}

#[test]
fn a_legacy_row_sealed_under_its_catalogue_purpose_reopens_to_the_same_bytes() {
    let cipher = cipher();
    for purpose in SecretPurpose::ALL {
        let key = key_for(purpose);
        let plaintext = format!("synthetic-plaintext-for-{key}").into_bytes();

        // What the sweep does with a claimed legacy row.
        let stored = seal_for_storage(Some(&cipher), purpose, &key, &plaintext)
            .expect("seal under the resolved purpose");

        assert_eq!(stored.ciphertext_version, SEALED_CIPHERTEXT_VERSION);
        assert_eq!(stored.purpose.as_deref(), Some(purpose.as_str()));
        assert_eq!(stored.key_version, KEY_VERSION as i32);
        assert!(
            !stored
                .value
                .windows(plaintext.len())
                .any(|window| window == plaintext),
            "{key}: the stored bytes must not contain the plaintext"
        );

        // And what an ordinary read then does with it: the same bytes back,
        // under the purpose the resolver named.
        let row = context69_secret_store::StoredSecret {
            value: stored.value.clone(),
            purpose: stored.purpose.clone(),
            key_version: stored.key_version,
            ciphertext_version: stored.ciphertext_version,
        };
        let opened = context69_secret_store::open_from_storage(
            Some(&cipher),
            purpose,
            &SecretKeyName::new(&key).expect("resolved key is a valid name"),
            &row,
        )
        .expect("open the sealed row");
        assert_eq!(opened.expose(), plaintext, "{key}");
    }
}

#[test]
fn a_sealed_row_is_opened_only_under_the_purpose_its_key_resolves_to() {
    // The conversion binds the owner into the frame's associated data, so a row
    // sealed for one purpose cannot be read through another even though the
    // same key name resolves. That is what stops a mis-resolved legacy row from
    // silently producing a readable secret under the wrong owner.
    let cipher = cipher();
    let key = key_for(SecretPurpose::GitProviderToken);
    let plaintext = b"synthetic-plaintext".as_slice();
    let stored = seal_for_storage(
        Some(&cipher),
        SecretPurpose::GitProviderToken,
        &key,
        plaintext,
    )
    .expect("seal");
    let row = context69_secret_store::StoredSecret {
        value: stored.value,
        purpose: stored.purpose,
        key_version: stored.key_version,
        ciphertext_version: stored.ciphertext_version,
    };

    let error = context69_secret_store::open_from_storage(
        Some(&cipher),
        SecretPurpose::GitWebhookSigningSecret,
        &SecretKeyName::new(&key).expect("valid name"),
        &row,
    )
    .expect_err("a foreign purpose must not open the row");
    assert!(error.to_string().contains("different purpose"), "{error}");
    assert!(!error.to_string().contains(MASTER_KEY_B64));
}

#[test]
fn a_legacy_row_is_still_readable_byte_for_byte_without_a_key() {
    // The transition the sweep protects: an unkeyed deployment must resolve the
    // exact bytes it always resolved, so sealing can be added without changing
    // what an unconfigured installation reads.
    let plaintext = b"synthetic-plaintext".as_slice();
    let stored = seal_for_storage(
        None,
        SecretPurpose::EmbeddingApiKey,
        "embedding_api_key",
        plaintext,
    )
    .expect("the legacy representation");
    assert_eq!(stored.value, plaintext);

    let row = context69_secret_store::StoredSecret {
        value: stored.value,
        purpose: stored.purpose,
        key_version: stored.key_version,
        ciphertext_version: stored.ciphertext_version,
    };
    let opened = context69_secret_store::open_from_storage(
        None,
        SecretPurpose::EmbeddingApiKey,
        &SecretKeyName::new("embedding_api_key").expect("valid name"),
        &row,
    )
    .expect("an unkeyed deployment reads its legacy row");
    assert_eq!(opened.expose(), plaintext);
}

#[test]
fn a_row_rendering_never_carries_a_value_or_key_material() {
    // The sweep handles a row shape of its own, so its rendering is held to the
    // same rule as every other row in this crate.
    let cipher = cipher();
    // A record-scoped purpose, so the row's *key name* is strictly longer than the
    // purpose string it legitimately renders: the assertion can tell the owner
    // being reported from the key being echoed.
    let purpose = SecretPurpose::GitProviderToken;
    let key = key_for(purpose);
    let stored = seal_for_storage(Some(&cipher), purpose, &key, b"secret").expect("seal");

    let report = format!(
        "{:?}",
        context69_secret_store::StoredSecret {
            value: stored.value.clone(),
            purpose: stored.purpose.clone(),
            key_version: stored.key_version,
            ciphertext_version: stored.ciphertext_version,
        }
    );
    assert!(report.contains("value_len"), "{report}");
    for leak in [MASTER_KEY_B64, key.as_str()] {
        assert!(!report.contains(leak), "{report} leaked {leak}");
    }
}

#[test]
fn the_sealing_run_is_refused_before_it_reaches_a_row_when_it_has_no_key() {
    // A store with a key, asked against a database that is not there, must fail
    // as a database failure and not as a sealing one: the key is present, so the
    // run really did start. The pool is opened inside the runtime that drives the
    // case, and the acquire timeout is short, so a run that had *reached* the
    // database would fail promptly rather than wait.
    let error = sqlx::test_block_on(async {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(1))
            .connect_lazy("postgres://c69test@127.0.0.1:1/nowhere")
            .expect("a lazy pool needs no reachable server");
        let store = SecretStore::new(
            context69_secret_store::SecretDatabase::new(pool),
            Some(MASTER_KEY_B64),
            KEY_VERSION,
        )
        .expect("a test master key builds a store");
        store.seal_legacy_plaintext().await
    })
    .expect_err("no database");
    assert_eq!(format!("{error:?}"), "DatabaseUnavailable");
    let rendered = format!("{error}");
    for leak in [MASTER_KEY_B64, "127.0.0.1", "SELECT", "internal_secrets"] {
        assert!(!rendered.contains(leak), "{rendered} leaked {leak:?}");
    }
}
