//! Database-free tests for the secret framing and cipher. Failures are asserted
//! on error *kinds* and on the absence of a secret in any rendered error, never
//! by printing a value.

use context69_secret_crypto::{
    FrameError, MasterKey, SecretCipher, SecretCipherError, SecretFrame, SecretPurpose,
    SecretValue, frame, frame_header, is_frame,
};

/// A fixed 32-byte test key: bytes `00..20`. A test constant, not a deployment
/// secret.
const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// A different fixed 32-byte test key: bytes `20..40`.
const OTHER_MASTER_KEY_B64: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";
const KEY_NAME: &str = "browser_session_signing_key_v2";
const PURPOSE: &str = "browser_session.signing_key";
const ASCII_SECRET: &[u8] = b"a-plaintext-secret-value-that-must-never-be-logged";
/// Names a frame header can never carry. The first is empty, the rest are
/// outside the identifier character set.
const BAD_NAMES: [&str; 8] = [
    "",
    "with space",
    "with\nnewline",
    "with\0nul",
    "with\"quote",
    "with\\backslash",
    "with/slash",
    "üñïçø∂é",
];

fn cipher(key_version: u32) -> SecretCipher {
    SecretCipher::new(master_key(MASTER_KEY_B64), key_version)
}

fn other_cipher(key_version: u32) -> SecretCipher {
    SecretCipher::new(master_key(OTHER_MASTER_KEY_B64), key_version)
}

fn master_key(encoded: &str) -> MasterKey {
    MasterKey::from_base64(encoded).expect("test master key")
}

fn purpose(name: &str) -> SecretPurpose {
    SecretPurpose::new(name).expect("test purpose")
}

fn seal_with(cipher: &SecretCipher, purpose_name: &str, key_name: &str) -> Vec<u8> {
    cipher
        .seal(&purpose(purpose_name), key_name, ASCII_SECRET)
        .expect("seal")
}

fn seal(cipher: &SecretCipher) -> Vec<u8> {
    seal_with(cipher, PURPOSE, KEY_NAME)
}

fn open(cipher: &SecretCipher, stored: &[u8]) -> SecretValue {
    cipher
        .open(&purpose(PURPOSE), KEY_NAME, stored)
        .expect("open")
}

fn open_err(cipher: &SecretCipher, stored: &[u8]) -> SecretCipherError {
    cipher
        .open(&purpose(PURPOSE), KEY_NAME, stored)
        .expect_err("must not open")
}

/// Whether a rejected name failed on its length or on its characters.
fn name_error(name: &str) -> FrameError {
    if name.is_empty() {
        FrameError::InvalidNameLength
    } else {
        FrameError::InvalidNameCharacters
    }
}

// --- round trip -------------------------------------------------------------

#[test]
fn round_trip_returns_the_exact_plaintext_without_echoing_it() {
    let cipher = cipher(1);
    for plaintext in [ASCII_SECRET, &[], &[0x5a_u8; 64], &[0xff_u8; 4096]] {
        let sealed = cipher
            .seal(&purpose(PURPOSE), KEY_NAME, plaintext)
            .expect("seal");
        assert!(is_frame(&sealed));
        assert!(
            !sealed
                .windows(plaintext.len().max(1))
                .any(|window| window == plaintext),
            "sealed bytes must not contain the plaintext"
        );
        assert_eq!(open(&cipher, &sealed).expose(), plaintext);
    }
}

#[test]
fn every_seal_draws_a_fresh_nonce() {
    let cipher = cipher(1);
    let first = seal(&cipher);
    let second = seal(&cipher);
    assert_ne!(first, second, "identical plaintext must not repeat a nonce");
    for stored in [&first, &second] {
        assert_eq!(open(&cipher, stored).expose(), ASCII_SECRET);
    }
}

// --- framing ----------------------------------------------------------------

#[test]
fn a_frame_round_trips_its_metadata_and_body() {
    let stored = seal(&cipher(3));
    let decoded = SecretFrame::decode(&stored).expect("decode");
    assert_eq!(decoded.format_version, frame::FORMAT_VERSION);
    assert_eq!(decoded.key_version, 3);
    assert_eq!(decoded.purpose, PURPOSE);
    assert_eq!(decoded.key_name, KEY_NAME);
    assert_eq!(
        decoded.ciphertext.len(),
        ASCII_SECRET.len() + frame::TAG_LEN
    );
    assert_eq!(decoded.encode(), stored, "encode is not canonical");
    // A seal grows the plaintext by exactly the authenticated header and the tag.
    let header = frame_header(3, PURPOSE, KEY_NAME, decoded.nonce).expect("header");
    assert_eq!(decoded.associated_data(), header);
    assert_eq!(
        decoded.encode().len(),
        ASCII_SECRET.len() + header.len() + frame::TAG_LEN
    );
    assert_eq!(decoded.metadata().key_version, 3);
    assert_eq!(
        SecretFrame::new(
            decoded.key_version,
            PURPOSE,
            KEY_NAME,
            decoded.nonce,
            decoded.ciphertext.clone()
        )
        .expect("rebuild"),
        decoded
    );
}

#[test]
fn legacy_plaintext_is_not_mistaken_for_a_frame() {
    let legacy = [0x42_u8; 64];
    assert!(!is_frame(&legacy));
    assert_eq!(
        SecretFrame::decode(&legacy).expect_err("legacy bytes"),
        FrameError::NotAFrame
    );
    assert_eq!(
        open_err(&cipher(1), &legacy),
        SecretCipherError::Frame(FrameError::NotAFrame)
    );
}

#[test]
fn an_unknown_container_version_is_refused_before_decryption() {
    let mut stored = seal(&cipher(1));
    stored[frame::MAGIC.len()] = frame::FORMAT_VERSION + 1;
    let expected = FrameError::UnsupportedFormatVersion(frame::FORMAT_VERSION + 1);
    assert_eq!(SecretFrame::decode(&stored).expect_err("future"), expected);
    assert_eq!(
        open_err(&cipher(1), &stored),
        SecretCipherError::Frame(expected)
    );
}

#[test]
fn no_proper_prefix_of_a_frame_can_be_opened() {
    let stored = seal(&cipher(1));
    for len in 0..stored.len() {
        let prefix = &stored[..len];
        // Short of the magic it is not a frame; longer it is a frame whose
        // ciphertext is truncated. Neither may open.
        if prefix.len() < frame::MAGIC.len() {
            let short = SecretFrame::decode(prefix).expect_err("short prefix");
            assert_eq!(short, FrameError::NotAFrame, "prefix of length {len}");
        }
        assert!(cipher(1).open(&purpose(PURPOSE), KEY_NAME, prefix).is_err());
    }
}

#[test]
fn a_frame_whose_ciphertext_is_only_a_partial_tag_is_refused() {
    let stored = cipher(1)
        .seal(&purpose(PURPOSE), KEY_NAME, &[])
        .expect("seal");
    let short = &stored[..stored.len() - 1];
    assert_eq!(
        SecretFrame::decode(short).expect_err("short tag"),
        FrameError::CiphertextTooShort,
        "a ciphertext must carry a whole tag"
    );
}

#[test]
fn a_name_outside_its_bounds_is_refused_with_the_right_reason() {
    let nonce = [0_u8; frame::NONCE_LEN];
    for name in BAD_NAMES {
        assert_eq!(
            frame_header(1, name, KEY_NAME, nonce).expect_err(name),
            name_error(name),
            "header purpose {name:?}"
        );
        assert_eq!(
            frame_header(1, PURPOSE, name, nonce).expect_err(name),
            name_error(name),
            "header key name {name:?}"
        );
        assert_eq!(
            SecretPurpose::new(name).expect_err(name),
            SecretCipherError::InvalidPurpose,
            "purpose {name:?}"
        );
        assert_eq!(
            cipher(1)
                .seal(&purpose(PURPOSE), name, ASCII_SECRET)
                .expect_err(name),
            SecretCipherError::Frame(name_error(name)),
            "seal under {name:?}"
        );
    }
    let too_long = "p".repeat(frame::MAX_PURPOSE_LEN + 1);
    assert_eq!(
        frame_header(1, &too_long, KEY_NAME, nonce).expect_err("long purpose"),
        FrameError::InvalidNameLength
    );
    assert_eq!(
        SecretPurpose::new(&too_long).expect_err("long purpose"),
        SecretCipherError::InvalidPurpose
    );
    let too_long = "k".repeat(frame::MAX_KEY_NAME_LEN + 1);
    assert_eq!(
        frame_header(1, PURPOSE, &too_long, nonce).expect_err("long key name"),
        FrameError::InvalidNameLength
    );
    // The documented bounds themselves are accepted.
    assert!(
        frame_header(
            1,
            &"p".repeat(frame::MAX_PURPOSE_LEN),
            &"k".repeat(frame::MAX_KEY_NAME_LEN),
            nonce
        )
        .is_ok()
    );
    assert!(SecretPurpose::new(&"p".repeat(frame::MAX_PURPOSE_LEN)).is_ok());
}

#[test]
fn a_stored_header_with_a_non_utf8_name_is_refused() {
    let mut stored = seal(&cipher(1));
    // First byte of the purpose field.
    stored[frame::MAGIC.len() + 1 + 4 + 2] = 0xff;
    assert_eq!(
        SecretFrame::decode(&stored).expect_err("non-utf8 purpose"),
        FrameError::InvalidNameEncoding
    );
}

#[test]
fn a_debug_rendering_of_a_frame_reports_metadata_but_not_the_ciphertext() {
    let decoded = SecretFrame::decode(&seal(&cipher(1))).expect("decode");
    let rendered = format!("{decoded:?}");
    for expected in [PURPOSE, KEY_NAME, "ciphertext_len"] {
        assert!(
            rendered.contains(expected),
            "{expected} missing: {rendered}"
        );
    }
    assert!(
        !rendered
            .as_bytes()
            .windows(8)
            .any(|window| window == &decoded.ciphertext[..8]),
        "a frame rendering must not embed the ciphertext"
    );
}

// --- binding ----------------------------------------------------------------

#[test]
fn a_secret_is_bound_to_its_purpose_and_key_name() {
    let cipher = cipher(1);
    let stored = seal(&cipher);
    assert_eq!(
        cipher
            .open(&purpose("git_credential.pat"), KEY_NAME, &stored)
            .expect_err("foreign purpose"),
        SecretCipherError::PurposeMismatch
    );
    assert_eq!(
        cipher
            .open(&purpose(PURPOSE), "git_pat", &stored)
            .expect_err("foreign key name"),
        SecretCipherError::KeyNameMismatch
    );
    // Mixed case is a valid identifier, not a normalization boundary.
    let mixed = "Git_Credential.App-Private-Key";
    let mixed_key = "GitHub.App.privateKey";
    assert_eq!(
        cipher
            .open(
                &purpose(mixed),
                mixed_key,
                &seal_with(&cipher, mixed, mixed_key)
            )
            .expect("open")
            .expose(),
        ASCII_SECRET
    );
}

#[test]
fn a_retired_master_key_version_is_reported_before_decryption() {
    let stored = seal(&cipher(1));
    assert_eq!(
        cipher(2)
            .open(&purpose(PURPOSE), KEY_NAME, &stored)
            .expect_err("retired key version"),
        SecretCipherError::KeyVersionMismatch {
            configured: 2,
            stored: 1,
        }
    );
}

#[test]
fn another_master_key_never_authenticates() {
    let stored = seal(&cipher(1));
    assert_eq!(
        open_err(&other_cipher(1), &stored),
        SecretCipherError::AuthenticationFailed
    );
}

#[test]
fn any_single_bit_flip_anywhere_in_the_frame_is_refused_without_leaking() {
    let stored = seal(&cipher(1));
    for index in 0..stored.len() {
        for bit in [0_u8, 3, 7] {
            let mut tampered = stored.clone();
            tampered[index] ^= 1 << bit;
            let rendered = open_err(&cipher(1), &tampered).to_string();
            assert!(
                !rendered.contains("a-plaintext") && !rendered.contains(MASTER_KEY_B64),
                "byte {index} bit {bit}: {rendered}"
            );
        }
    }
}

#[test]
fn appending_bytes_to_a_frame_is_refused() {
    let mut stored = seal(&cipher(1));
    stored.extend_from_slice(&[0_u8; 32]);
    assert_eq!(
        open_err(&cipher(1), &stored),
        SecretCipherError::AuthenticationFailed
    );
}

// --- master key handling ----------------------------------------------------

#[test]
fn a_master_key_parses_with_and_without_padding() {
    let unpadded = SecretCipher::new(master_key(MASTER_KEY_B64.trim_end_matches('=')), 1);
    let stored = seal(&cipher(1));
    assert_eq!(open(&unpadded, &stored).expose(), ASCII_SECRET);
    assert!(MasterKey::from_base64(&format!("  {MASTER_KEY_B64}  ")).is_ok());
    assert!(MasterKey::from_bytes(&[7_u8; 32]).is_ok());
}

#[test]
fn a_master_key_of_the_wrong_length_is_refused() {
    for (value, len) in [
        ("AAECAw==", 4_usize),
        ("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHg==", 31),
        ("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g", 33),
        ("A".repeat(64).as_str(), 48),
    ] {
        assert_eq!(
            MasterKey::from_base64(value).expect_err("wrong length"),
            SecretCipherError::InvalidMasterKeyLength(len),
            "{value}"
        );
    }
    for len in [16_usize, 64] {
        assert_eq!(
            MasterKey::from_bytes(&vec![0_u8; len]).expect_err("raw bytes"),
            SecretCipherError::InvalidMasterKeyLength(len)
        );
    }
}

#[test]
fn a_master_key_that_is_not_base64_is_refused_without_echoing_it() {
    let rejected = "not a base64 master key !!";
    let error = MasterKey::from_base64(rejected).expect_err("not base64");
    assert_eq!(error, SecretCipherError::InvalidMasterKeyEncoding);
    assert!(!error.to_string().contains(rejected));
    assert!(!format!("{error:?}").contains(rejected));
}

#[test]
fn key_material_and_plaintext_are_never_rendered() {
    assert_eq!(
        format!("{:?}", master_key(MASTER_KEY_B64)),
        "MasterKey(<redacted>)"
    );
    let cipher = SecretCipher::new(master_key(MASTER_KEY_B64), 7);
    let rendered = format!("{cipher:?}");
    assert!(rendered.contains("key_version: 7"), "{rendered}");
    assert!(!rendered.contains(MASTER_KEY_B64), "{rendered}");
    assert_eq!(cipher.key_version(), 7);

    let opened = open(&cipher, &seal(&cipher));
    assert!(!format!("{opened:?}").contains("a-plaintext-secret"));
    assert_eq!(opened.expose(), ASCII_SECRET);
    assert_eq!(opened.into_inner(), ASCII_SECRET);
}
