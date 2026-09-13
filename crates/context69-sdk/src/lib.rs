mod client;
mod error;
pub mod raw;
pub mod raw_operations;

pub use client::*;
pub use error::Error;
pub use raw::{RawClient, RawRequest, RawResponse, find_operation, operation_ids};
pub use raw_operations::{BodyKind, OPERATIONS, Operation};
