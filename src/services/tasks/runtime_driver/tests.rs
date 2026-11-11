//! Pure claim-run kind units.

use super::parse_kind;
use crate::domain_errors::{DomainError, find_domain_error};

#[test]
fn unknown_task_kind_is_typed_invalid_argument() {
    let Err(error) = parse_kind("bogus") else {
        panic!("expected unknown task kind to fail");
    };
    assert!(matches!(
        find_domain_error(&error),
        Some(DomainError::InvalidArgument(_))
    ));
    assert_eq!(error.to_string(), "unsupported task kind bogus");
}
