//! Typed domain error taxonomy shared by API mappers and service producers.
//!
//! The canonical [`DomainError`](context69_contracts::DomainError) enum lives
//! in `context69-contracts` so the independent `context69-search` and
//! `context69-translation` crates can construct it without depending on HTTP
//! types. HTTP status mapping lives in `context69-http-support`, re-exported
//! here alongside the enum for service code.

pub use context69_contracts::DomainError;
pub use context69_http_support::{
    code_for_error, domain_status, find_domain_error, is_invalid_argument_error,
    is_not_found_error, status_for_error, typed_status_for_error,
};
