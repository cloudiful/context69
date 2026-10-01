use super::{BoundedBase64, ByteBudget, check_declared_length};
use crate::domain_errors::DomainError;

fn budget(max_bytes: usize) -> ByteBudget {
    ByteBudget { max_bytes }
}

fn decode_all(encoded: &str, max_bytes: usize) -> Result<Vec<u8>, anyhow::Error> {
    let mut decoder = BoundedBase64::new(budget(max_bytes));
    let mut decoded = decoder.push_chunk(encoded.as_bytes())?;
    decoded.extend(decoder.finish()?);
    Ok(decoded)
}
#[test]
fn decodes_single_chunk_and_split_chunks_identically() {
    // 48 'a' bytes -> 64 base64 chars, wrapped like GitHub does.
    let payload = vec![b'a'; 48];
    let encoded = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&payload)
    };
    let wrapped: String = encoded
        .as_bytes()
        .chunks(60)
        .flat_map(|chunk| {
            let mut line = chunk.to_vec();
            line.push(b'\n');
            line
        })
        .map(|byte| byte as char)
        .collect();
    assert_eq!(decode_all(&encoded, 1024).unwrap(), payload);
    assert_eq!(decode_all(&wrapped, 1024).unwrap(), payload);

    let bytes = encoded.as_bytes();
    for split_at in 1..bytes.len() {
        let mut decoder = BoundedBase64::new(budget(1024));
        let mut decoded = decoder.push_chunk(&bytes[..split_at]).unwrap();
        decoded.extend(decoder.push_chunk(&bytes[split_at..]).unwrap());
        decoded.extend(decoder.finish().unwrap());
        assert_eq!(decoded, payload, "split at offset {split_at} must decode");
    }
}

#[test]
fn rejects_trailing_bits_in_the_final_symbol() {
    // "Zg==" is the canonical encoding of b"f"; "Zh==" sets the unused
    // trailing bits of the final symbol and must be rejected rather than
    // silently decoded to the same byte.
    assert_eq!(decode_all("Zg==", 1024).unwrap(), b"f");
    assert!(decode_all("Zh==", 1024).is_err());
}

#[test]
fn rejects_non_base64_and_mid_stream_padding() {
    assert!(decode_all("not*base64!", 1024).is_err());
    // Data after padding is never valid, even split across chunks.
    assert!(decode_all("aGVsbG8=aGVsbG8=", 1024).is_err());
    let mut decoder = BoundedBase64::new(budget(1024));
    decoder.push_chunk(b"aGVs").unwrap();
    assert!(decoder.push_chunk(b"bG8=").is_ok());
    assert!(decoder.push_chunk(b"aGVs").is_err());
    // Leading padding is invalid.
    let mut decoder = BoundedBase64::new(budget(1024));
    assert!(decoder.push_chunk(b"=GVs").is_err());
}

#[test]
fn rejects_truncated_final_group_without_padding() {
    // "aGVsbG8" is a valid 4-group + 3-tail; without padding the tail must
    // not decode.
    let mut decoder = BoundedBase64::new(budget(1024));
    decoder.push_chunk(b"aGVsbG8").unwrap();
    assert!(decoder.finish().is_err());
}

#[test]
fn charges_encoded_and_decoded_bytes_against_one_budget() {
    // 60 decoded bytes of patterned data -> 80 canonical-padded encoded
    // bytes; the decoder must charge both sides against one budget.
    let payload: Vec<u8> = (0..60u32).map(|index| (index % 251) as u8).collect();
    let encoded = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&payload)
    };
    assert_eq!(encoded.len(), 80);
    let mut decoder = BoundedBase64::new(budget(200));
    let mut decoded = Vec::new();
    for chunk in encoded.as_bytes().chunks(8) {
        decoded.extend(decoder.push_chunk(chunk).unwrap());
    }
    // 80 encoded + 60 decoded, all charged during pushes.
    assert_eq!(decoder.spent(), 140);
    decoded.extend(decoder.finish().unwrap());
    assert_eq!(decoded, payload);
    assert_eq!(decoder.spent(), 140);
}

#[test]
fn aggregate_budget_fails_closed_across_chunks() {
    let payload = vec![7u8; 200];
    let encoded = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&payload)
    };
    let mut decoder = BoundedBase64::new(budget(100));
    let mut decoded = Vec::new();
    let mut failed = false;
    for chunk in encoded.as_bytes().chunks(16) {
        match decoder.push_chunk(chunk) {
            Ok(part) => decoded.extend(part),
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    assert!(failed, "budget must trip before the stream completes");
    assert!(decoded.len() < 200);
}

#[test]
fn declared_length_gate_rejects_oversized_declarations() {
    let limits = budget(1024);
    assert!(check_declared_length(None, limits).is_ok());
    assert!(check_declared_length(Some(1024), limits).is_ok());
    assert!(check_declared_length(Some(1025), limits).is_err());
    assert!(check_declared_length(Some(u64::from(u32::MAX)), limits).is_err());
}

#[test]
fn budget_errors_are_payload_too_large_domain_errors() {
    let error = check_declared_length(Some(9), budget(8)).unwrap_err();
    let domain = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<DomainError>())
        .unwrap();
    assert!(matches!(domain, DomainError::PayloadTooLarge(_)));
}
