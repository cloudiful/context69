//! Behavioural tests for the encrypted secret store's transition logic.
//!
//! Everything except the database round trip is a pure function of the stored
//! row, which is what this file exercises: a deployment with a master key, one
//! without, a row still in the legacy plaintext representation, and a row whose
//! bytes were tampered with. No test needs PostgreSQL, so none can leave a
//! fixture behind, and none prints a value from a real deployment. The
//! database-bound lifecycle is covered by the opt-in startup regression, which
//! drives the same browser-session path against a migrated scratch database.

use context69::{
    config::SecretStoreConfig,
    db::StoredInternalSecret,
    services::secret_store::{
        LEGACY_PLAINTEXT_VERSION, SEALED_CIPHERTEXT_VERSION, SecretPurpose, SecretStoreError,
        StoredSecretBytes, master_key_from_config, open_from_storage, seal_for_storage,
    },
};
use context69_secret_crypto::{MasterKey, SecretCipher, SecretCipherError, frame};

/// A fixed 32-byte test key: bytes `00..20`. A test constant.
const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const KEY_NAME: &str = "browser_session_signing_key_v2";
const SECRET: &[u8] = b"a-plaintext-secret-value-that-must-never-be-logged";
const SIGNING_KEY: &[u8] = &[0x5a_u8; 64];

fn cipher(key_version: u32) -> SecretCipher {
    let master = MasterKey::from_base64(MASTER_KEY_B64).expect("test master key");
    SecretCipher::new(master, key_version)
}

fn configured(key_version: u32) -> SecretStoreConfig {
    SecretStoreConfig {
        master_key: Some(MASTER_KEY_B64.to_string()),
        key_version,
    }
}

/// The store's error kind as a comparable label.
///
/// The error enum deliberately has no `PartialEq` (it wraps `anyhow::Error`),
/// so tests assert on the kind and read the rendered message separately.
fn kind(error: &SecretStoreError) -> &'static str {
    match error {
        SecretStoreError::MasterKeyNotConfigured => "master_key_not_configured",
        SecretStoreError::MasterKeyRejected(_) => "master_key_rejected",
        SecretStoreError::InvalidKeyName => "invalid_key_name",
        SecretStoreError::PurposeMismatch => "purpose_mismatch",
        SecretStoreError::SecretNotFound => "secret_not_found",
        SecretStoreError::Cipher(_) => "cipher",
        SecretStoreError::Database(_) => "database",
    }
}

fn legacy_row(value: &[u8]) -> StoredInternalSecret {
    StoredInternalSecret {
        value: value.to_vec(),
        purpose: None,
        key_version: LEGACY_PLAINTEXT_VERSION,
        ciphertext_version: LEGACY_PLAINTEXT_VERSION,
    }
}

fn sealed_row(stored: &StoredSecretBytes) -> StoredInternalSecret {
    StoredInternalSecret {
        value: stored.value.clone(),
        purpose: stored.purpose.clone(),
        key_version: stored.key_version,
        ciphertext_version: stored.ciphertext_version,
    }
}

fn seal(cipher: Option<&SecretCipher>) -> StoredSecretBytes {
    seal_for_storage(
        cipher,
        SecretPurpose::BrowserSessionSigningKey,
        KEY_NAME,
        SIGNING_KEY,
    )
    .expect("seal")
}

// --- configuration ----------------------------------------------------------

#[test]
fn an_unconfigured_deployment_has_no_master_key() {
    for absent in [
        SecretStoreConfig::default(),
        SecretStoreConfig {
            master_key: None,
            key_version: 1,
        },
        SecretStoreConfig {
            master_key: Some("   ".to_string()),
            key_version: 1,
        },
    ] {
        assert!(
            master_key_from_config(&absent)
                .expect("unconfigured")
                .is_none(),
            "an absent or blank key must be the unconfigured state"
        );
    }
}

#[test]
fn a_configured_deployment_yields_a_master_key_and_never_echoes_it() {
    let master = master_key_from_config(&configured(3))
        .expect("configured")
        .expect("master key");

    assert_eq!(format!("{master:?}"), "MasterKey(<redacted>)");
}

#[test]
fn an_unusable_master_key_is_a_configuration_failure() {
    for rejected in [
        "AAECAw==".to_string(),
        "not base64 !!".to_string(),
        "A".repeat(64),
    ] {
        let config = SecretStoreConfig {
            master_key: Some(rejected.clone()),
            key_version: 1,
        };
        let error = master_key_from_config(&config).expect_err("unusable key");
        assert_eq!(kind(&error), "master_key_rejected", "rejected {rejected:?}");
        assert!(
            !error.to_string().contains(&rejected),
            "the error must not echo the rejected key"
        );
    }
}

// --- writing ----------------------------------------------------------------

#[test]
fn a_configured_deployment_seals_with_purpose_and_version_metadata() {
    let stored = seal(Some(&cipher(3)));

    assert_eq!(stored.ciphertext_version, SEALED_CIPHERTEXT_VERSION);
    assert_eq!(stored.key_version, 3);
    assert_eq!(
        stored.purpose.as_deref(),
        Some(SecretPurpose::BrowserSessionSigningKey.as_str())
    );
    assert!(
        !stored
            .value
            .windows(SIGNING_KEY.len())
            .any(|window| window == SIGNING_KEY),
        "the stored bytes must not contain the secret"
    );
    assert!(frame::is_frame(&stored.value));
}

#[test]
fn an_unconfigured_deployment_still_creates_a_legacy_row() {
    let stored = seal(None);

    assert_eq!(stored.value, SIGNING_KEY);
    assert_eq!(stored.ciphertext_version, LEGACY_PLAINTEXT_VERSION);
    assert_eq!(stored.key_version, LEGACY_PLAINTEXT_VERSION);
    assert_eq!(stored.purpose, None);
}

#[test]
fn a_key_name_no_frame_could_carry_is_refused_before_anything_is_stored() {
    for cipher in [None, Some(cipher(1))] {
        for rejected in [
            "",
            "with space",
            "with\nnewline",
            "with/slash",
            "with:colon",
        ] {
            let error = seal_for_storage(
                cipher.as_ref(),
                SecretPurpose::BrowserSessionSigningKey,
                rejected,
                SECRET,
            )
            .expect_err(rejected);
            assert_eq!(kind(&error), "invalid_key_name", "key name {rejected:?}");
        }
        let error = seal_for_storage(
            cipher.as_ref(),
            SecretPurpose::BrowserSessionSigningKey,
            &"k".repeat(frame::MAX_KEY_NAME_LEN + 1),
            SECRET,
        )
        .expect_err("oversized key name");
        assert_eq!(kind(&error), "invalid_key_name");
    }
}

#[test]
fn the_bounded_key_name_is_accepted() {
    let key = "k".repeat(frame::MAX_KEY_NAME_LEN);
    assert!(
        seal_for_storage(
            Some(&cipher(1)),
            SecretPurpose::BrowserSessionSigningKey,
            &key,
            SECRET
        )
        .is_ok()
    );
}

// --- reading ----------------------------------------------------------------

#[test]
fn a_legacy_row_is_returned_byte_for_byte() {
    // The transition promise: a deployment that has not migrated yet resolves
    // the exact bytes it always resolved, so no installed signing key changes.
    let opened = open_from_storage(
        None,
        SecretPurpose::BrowserSessionSigningKey,
        KEY_NAME,
        &legacy_row(SIGNING_KEY),
    )
    .expect("legacy read");

    assert_eq!(opened.expose(), SIGNING_KEY);
}

#[test]
fn a_sealed_row_round_trips_through_the_store() {
    let cipher = cipher(1);
    let row = sealed_row(&seal(Some(&cipher)));

    let opened = open_from_storage(
        Some(&cipher),
        SecretPurpose::BrowserSessionSigningKey,
        KEY_NAME,
        &row,
    )
    .expect("open");
    assert_eq!(opened.expose(), SIGNING_KEY);
}

#[test]
fn a_sealed_row_never_degrades_to_its_ciphertext_or_a_default() {
    let cipher = cipher(1);
    let row = sealed_row(&seal(Some(&cipher)));

    let error = open_from_storage(
        None,
        SecretPurpose::BrowserSessionSigningKey,
        KEY_NAME,
        &row,
    )
    .expect_err("no configured key");
    assert_eq!(kind(&error), "master_key_not_configured");
    assert!(error.to_string().contains("secret_store.master_key"));
}

#[test]
fn a_sealed_row_is_refused_under_a_different_purpose() {
    let cipher = cipher(1);
    let mut row = sealed_row(&seal(Some(&cipher)));
    // A row claimed by another owner is refused on the stored metadata, before
    // the AEAD is consulted.
    row.purpose = Some("git_credential.pat".to_string());

    let error = open_from_storage(
        Some(&cipher),
        SecretPurpose::BrowserSessionSigningKey,
        KEY_NAME,
        &row,
    )
    .expect_err("foreign purpose");
    assert_eq!(kind(&error), "purpose_mismatch");
}

#[test]
fn a_sealed_row_is_refused_under_a_different_key_name() {
    let cipher = cipher(1);
    let row = sealed_row(&seal(Some(&cipher)));

    let error = open_from_storage(
        Some(&cipher),
        SecretPurpose::BrowserSessionSigningKey,
        "git_pat",
        &row,
    )
    .expect_err("foreign key name");
    assert_eq!(kind(&error), "cipher");
    assert!(matches!(
        open_from_storage(
            Some(&cipher),
            SecretPurpose::BrowserSessionSigningKey,
            "git_pat",
            &row
        ),
        Err(SecretStoreError::Cipher(SecretCipherError::KeyNameMismatch))
    ));
}

#[test]
fn a_retired_master_key_version_is_not_silently_mis_decrypted() {
    let row = sealed_row(&seal(Some(&cipher(1))));

    assert!(matches!(
        open_from_storage(
            Some(&cipher(2)),
            SecretPurpose::BrowserSessionSigningKey,
            KEY_NAME,
            &row
        ),
        Err(SecretStoreError::Cipher(
            SecretCipherError::KeyVersionMismatch {
                configured: 2,
                stored: 1,
            }
        ))
    ));
}

#[test]
fn a_tampered_sealed_row_never_yields_a_value() {
    let cipher = cipher(1);
    let stored = seal(Some(&cipher));

    for index in 0..stored.value.len() {
        let mut tampered = stored.value.clone();
        tampered[index] ^= 0x01;
        let row = StoredInternalSecret {
            value: tampered,
            purpose: stored.purpose.clone(),
            key_version: stored.key_version,
            ciphertext_version: stored.ciphertext_version,
        };
        let error = open_from_storage(
            Some(&cipher),
            SecretPurpose::BrowserSessionSigningKey,
            KEY_NAME,
            &row,
        )
        .expect_err("tampered row must not open");
        assert!(
            !error.to_string().contains("a-plaintext"),
            "byte {index}: {}",
            error
        );
    }
}

#[test]
fn a_row_marked_sealed_is_never_handed_back_as_plaintext() {
    let cipher = cipher(1);
    let stored = seal(Some(&cipher));
    // Legacy bytes with a sealed marker: the marker is authoritative, so the
    // value is not returned as if it were plaintext.
    let forged = StoredInternalSecret {
        value: SIGNING_KEY.to_vec(),
        purpose: stored.purpose.clone(),
        key_version: stored.key_version,
        ciphertext_version: SEALED_CIPHERTEXT_VERSION,
    };
    let error = open_from_storage(
        Some(&cipher),
        SecretPurpose::BrowserSessionSigningKey,
        KEY_NAME,
        &forged,
    )
    .expect_err("forged marker");
    assert_eq!(kind(&error), "cipher");
}

#[test]
fn the_legacy_marker_is_the_only_plaintext_path() {
    let row = legacy_row(SIGNING_KEY);
    assert!(row.is_legacy_plaintext());
    assert_eq!(row.purpose, None);
    assert_eq!(row.key_version, LEGACY_PLAINTEXT_VERSION);
    // A sealed row is never reported as legacy, whatever its bytes look like.
    assert!(!sealed_row(&seal(Some(&cipher(1)))).is_legacy_plaintext());
    // A row rendering must not embed the stored bytes.
    let rendered = format!("{row:?}");
    assert!(rendered.contains("value_len"), "{rendered}");
    assert!(
        !rendered
            .as_bytes()
            .windows(8)
            .any(|window| window == &SIGNING_KEY[..8]),
        "{rendered}"
    );
}

// --- purpose ownership ------------------------------------------------------

#[test]
fn the_browser_session_purpose_is_namespaced() {
    assert_eq!(
        SecretPurpose::BrowserSessionSigningKey.as_str(),
        "browser_session.signing_key"
    );
    assert_eq!(
        SecretPurpose::BrowserSessionSigningKey.to_string(),
        "browser_session.signing_key"
    );
    // A purpose is sealed into the frame, so it must be a name the framing
    // layer accepts.
    assert!(frame::is_valid_purpose(
        SecretPurpose::BrowserSessionSigningKey.as_str()
    ));
    assert!(frame::is_valid_key_name(KEY_NAME));
}
