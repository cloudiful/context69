//! The local mock wire tests of the Docling client.
//!
//! Every test runs against a local HTTP server on `127.0.0.1:0`; no test
//! ever reaches a deployed Docling Serve.

mod client_ops;
mod client_submit;
mod config;
mod management;
mod models;
mod support;
mod task_id;
