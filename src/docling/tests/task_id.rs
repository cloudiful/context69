//! Task id validation and encoding.

use crate::docling::task_id::{MAX_TASK_ID_BYTES, encode_task_id_segment, validate_task_id};

#[test]
fn task_ids_are_percent_encoded_without_changing_the_path() {
    assert_eq!(encode_task_id_segment("abc-123_UUID.~"), "abc-123_UUID.~");
    assert_eq!(encode_task_id_segment("a/b?c#d"), "a%2Fb%3Fc%23d");
    assert_eq!(encode_task_id_segment("a b"), "a%20b");
}

#[test]
fn a_unicode_task_id_is_encoded_byte_by_byte() {
    assert_eq!(encode_task_id_segment("täst"), "t%C3%A4st");
    assert_eq!(encode_task_id_segment("日本"), "%E6%97%A5%E6%9C%AC");
}

#[test]
fn task_id_validation_rejects_empty_overlong_and_whitespace() {
    assert!(validate_task_id("").is_err());
    assert!(validate_task_id("   ").is_err());
    assert!(validate_task_id("a b").is_err());
    assert!(validate_task_id("a\nb").is_err());
    let long = "x".repeat(MAX_TASK_ID_BYTES + 1);
    assert!(validate_task_id(&long).is_err());
    assert!(validate_task_id(&"x".repeat(MAX_TASK_ID_BYTES)).is_ok());
    assert!(validate_task_id("task-123").is_ok());
}

#[test]
fn the_task_id_bound_is_measured_in_bytes() {
    // A multi-byte id reaches the bound before it reaches the character count.
    let id = "é".repeat(MAX_TASK_ID_BYTES / 2 + 1);
    assert!(id.len() > MAX_TASK_ID_BYTES / 2);
    assert!(validate_task_id(&id).is_err());
}
