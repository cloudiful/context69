//! The store's transition semantics, without a database.
//!
//! Everything except the SQL round trip is a pure function of a stored row, so
//! the rules that decide whether a value may be returned — legacy versus sealed,
//! configured versus unconfigured, this purpose versus another — are asserted
//! here directly. No test needs PostgreSQL, so none can leave a fixture behind,
//! and none prints a value that came from a real deployment.
//!
//! The base64 key below is a test constant (bytes `00..20`), not a deployment
//! secret.

use context69_secret_store::crypto::{MasterKey, SecretCipher, SecretCipherError};
use context69_secret_store::{
    LEGACY_PLAINTEXT_VERSION, SEALED_CIPHERTEXT_VERSION, SecretKeyName, SecretPurpose,
    SecretStoreError, StoredSecret, StoredSecretBytes, StoredSecretMetadata, key_names,
    master_key_from_config, open_from_storage, seal_for_storage,
};

const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const BROWSER: SecretPurpose = SecretPurpose::BrowserSessionSigningKey;
const SIGNING_KEY: &[u8] = &[0x5a_u8; 64];

fn key(name: &str) -> SecretKeyName {
    SecretKeyName::new(name).expect("valid key name")
}

fn browser_key() -> SecretKeyName {
    key(key_names::BROWSER_SESSION_SIGNING_KEY)
}

fn cipher(key_version: u32) -> SecretCipher {
    SecretCipher::new(
        MasterKey::from_base64(MASTER_KEY_B64).expect("test master key"),
        key_version,
    )
}

fn seal(with_cipher: Option<&SecretCipher>) -> StoredSecretBytes {
    seal_for_storage(
        with_cipher,
        BROWSER,
        key_names::BROWSER_SESSION_SIGNING_KEY,
        SIGNING_KEY,
    )
    .expect("seal")
}

fn row(stored: &StoredSecretBytes) -> StoredSecret {
    StoredSecret {
        value: stored.value.clone(),
        purpose: stored.purpose.clone(),
        key_version: stored.key_version,
        ciphertext_version: stored.ciphertext_version,
    }
}

fn legacy_row() -> StoredSecret {
    StoredSecret {
        value: SIGNING_KEY.to_vec(),
        purpose: None,
        key_version: LEGACY_PLAINTEXT_VERSION,
        ciphertext_version: LEGACY_PLAINTEXT_VERSION,
    }
}

fn open(
    with_cipher: Option<&SecretCipher>,
    stored: &StoredSecret,
) -> Result<Vec<u8>, SecretStoreError> {
    open_from_storage(with_cipher, BROWSER, &browser_key(), stored)
        .map(|value| value.expose().to_vec())
}

// --- configuration ----------------------------------------------------------

#[test]
fn an_unconfigured_deployment_has_no_master_key() {
    for absent in [None, Some(""), Some("   ")] {
        assert!(
            master_key_from_config(absent)
                .expect("unconfigured")
                .is_none(),
            "an absent or blank key must be the unconfigured state"
        );
    }
    assert!(
        master_key_from_config(Some(MASTER_KEY_B64))
            .expect("configured")
            .is_some()
    );
}

#[test]
fn an_unusable_master_key_is_a_configuration_failure_that_echoes_nothing() {
    for rejected in [
        "AAECAw==".to_string(),
        "not base64 !!".to_string(),
        "A".repeat(64),
    ] {
        let error = master_key_from_config(Some(&rejected)).expect_err("unusable key");
        assert!(
            matches!(error, SecretStoreError::MasterKeyRejected(_)),
            "rejected {rejected:?}"
        );
        assert!(!error.to_string().contains(&rejected));
    }
}

// --- writing ----------------------------------------------------------------

#[test]
fn a_configured_deployment_seals_with_purpose_and_version_metadata() {
    let stored = seal(Some(&cipher(3)));

    assert_eq!(stored.ciphertext_version, SEALED_CIPHERTEXT_VERSION);
    assert_eq!(stored.key_version, 3);
    assert_eq!(stored.purpose.as_deref(), Some(BROWSER.as_str()));
    assert!(
        !stored
            .value
            .windows(SIGNING_KEY.len())
            .any(|window| window == SIGNING_KEY),
        "the stored bytes must not contain the secret"
    );
    assert!(context69_secret_store::crypto::frame::is_frame(
        &stored.value
    ));
}

#[test]
fn an_unconfigured_deployment_still_creates_a_legacy_row() {
    let stored = seal(None);

    assert_eq!(stored.value, SIGNING_KEY);
    assert_eq!(stored.ciphertext_version, LEGACY_PLAINTEXT_VERSION);
    assert_eq!(stored.key_version, LEGACY_PLAINTEXT_VERSION);
    assert_eq!(stored.purpose, None);
}

// --- reading ----------------------------------------------------------------

#[test]
fn a_legacy_row_is_returned_byte_for_byte() {
    // The transition promise: a deployment that has not migrated resolves the
    // exact bytes it always resolved, so no installed signing key changes.
    assert_eq!(open(None, &legacy_row()).expect("legacy read"), SIGNING_KEY);
}

#[test]
fn a_sealed_row_round_trips_through_the_store() {
    let cipher = cipher(1);
    assert_eq!(
        open(Some(&cipher), &row(&seal(Some(&cipher)))).expect("open"),
        SIGNING_KEY
    );
}

#[test]
fn a_sealed_row_never_degrades_to_its_ciphertext_or_a_default() {
    let cipher = cipher(1);
    let sealed = row(&seal(Some(&cipher)));

    let error = open(None, &sealed).expect_err("no configured key");
    assert!(matches!(error, SecretStoreError::MasterKeyNotConfigured));
    assert!(error.to_string().contains("app.master_secret"));
}

#[test]
fn a_sealed_row_is_refused_under_a_different_purpose() {
    let cipher = cipher(1);
    let mut sealed = row(&seal(Some(&cipher)));
    // A row claimed by another owner is refused on the stored metadata, before
    // the AEAD is consulted.
    sealed.purpose = Some(SecretPurpose::GitProviderToken.as_str().to_string());

    let error = open_from_storage(Some(&cipher), BROWSER, &browser_key(), &sealed)
        .expect_err("foreign purpose");
    assert!(matches!(error, SecretStoreError::PurposeMismatch));
}

#[test]
fn a_sealed_row_is_refused_under_a_different_key_name() {
    let cipher = cipher(1);
    let sealed = row(&seal(Some(&cipher)));

    let error = open_from_storage(
        Some(&cipher),
        BROWSER,
        &key(key_names::EMBEDDING_API_KEY),
        &sealed,
    )
    .expect_err("foreign key name");
    assert!(matches!(
        error,
        SecretStoreError::Cipher(SecretCipherError::KeyNameMismatch)
    ));
}

#[test]
fn a_retired_master_key_version_is_not_silently_mis_decrypted() {
    let sealed = row(&seal(Some(&cipher(1))));

    let error = open_from_storage(Some(&cipher(2)), BROWSER, &browser_key(), &sealed)
        .expect_err("retired key version");
    assert!(matches!(
        error,
        SecretStoreError::Cipher(SecretCipherError::KeyVersionMismatch {
            configured: 2,
            stored: 1,
        })
    ));
}

#[test]
fn a_tampered_sealed_row_never_yields_a_value() {
    let cipher = cipher(1);
    let stored = seal(Some(&cipher));

    for index in 0..stored.value.len() {
        let mut tampered = stored.value.clone();
        tampered[index] ^= 0x01;
        let forged = StoredSecret {
            value: tampered,
            purpose: stored.purpose.clone(),
            key_version: stored.key_version,
            ciphertext_version: stored.ciphertext_version,
        };
        let error = open_from_storage(Some(&cipher), BROWSER, &browser_key(), &forged)
            .expect_err("tampered row must not open");
        assert!(!error.to_string().contains("a-plaintext"), "byte {index}");
    }
}

#[test]
fn a_row_marked_sealed_is_never_handed_back_as_plaintext() {
    let cipher = cipher(1);
    let stored = seal(Some(&cipher));
    // Legacy bytes with a sealed marker: the marker is authoritative, so the
    // value is not returned as if it were plaintext.
    let forged = StoredSecret {
        value: SIGNING_KEY.to_vec(),
        purpose: stored.purpose,
        key_version: stored.key_version,
        ciphertext_version: SEALED_CIPHERTEXT_VERSION,
    };
    assert!(matches!(
        open(Some(&cipher), &forged),
        Err(SecretStoreError::Cipher(_))
    ));
}

#[test]
fn a_row_rendering_never_embeds_the_stored_bytes() {
    let stored = seal(Some(&cipher(1)));
    let rendered = format!("{:?}", row(&stored));
    assert!(rendered.contains("value_len"), "{rendered}");
    assert!(!rendered.contains("C69S"), "{rendered}");
    assert!(!rendered.contains(MASTER_KEY_B64), "{rendered}");
    assert!(
        !rendered
            .as_bytes()
            .windows(8)
            .any(|window| window == &stored.value[..8]),
        "{rendered}"
    );
}

// --- metadata-only presence -------------------------------------------------

#[test]
fn metadata_presence_needs_no_master_key() {
    // The point of the metadata-only path: `has_*` stays truthful on a
    // deployment that cannot decrypt, and answering it decrypts nothing.
    let sealed = StoredSecretMetadata {
        purpose: Some(BROWSER.as_str().to_string()),
        ciphertext_version: SEALED_CIPHERTEXT_VERSION,
    };
    assert!(sealed.belongs_to(BROWSER.as_str()));
    assert!(!sealed.is_legacy_plaintext());
}

#[test]
fn metadata_presence_is_purpose_scoped_for_a_sealed_row() {
    let sealed = StoredSecretMetadata {
        purpose: Some(SecretPurpose::GitProviderToken.as_str().to_string()),
        ciphertext_version: SEALED_CIPHERTEXT_VERSION,
    };
    assert!(sealed.belongs_to(SecretPurpose::GitProviderToken.as_str()));
    assert!(!sealed.belongs_to(BROWSER.as_str()));
    assert!(!sealed.belongs_to(SecretPurpose::SearchApiKey.as_str()));
}

#[test]
fn metadata_presence_treats_a_legacy_row_as_unclaimed() {
    let legacy = StoredSecretMetadata {
        purpose: None,
        ciphertext_version: LEGACY_PLAINTEXT_VERSION,
    };
    assert!(legacy.is_legacy_plaintext());
    // The transition hands a legacy row back to the purpose that created it, so
    // presence must not report it as missing.
    for purpose in SecretPurpose::ALL {
        assert!(legacy.belongs_to(purpose.as_str()), "{purpose}");
    }
}

#[test]
fn a_value_row_reports_the_same_presence_rule_as_its_metadata() {
    let cipher = cipher(1);
    let stored = seal(Some(&cipher));
    let value_row = row(&stored);
    assert_eq!(
        value_row.metadata().belongs_to(BROWSER.as_str()),
        value_row.metadata().belongs_to(BROWSER.as_str())
    );
    assert!(value_row.metadata().belongs_to(BROWSER.as_str()));
    assert!(
        !value_row
            .metadata()
            .belongs_to(SecretPurpose::SearchApiKey.as_str())
    );
    assert!(legacy_row().metadata().belongs_to(BROWSER.as_str()));
}
