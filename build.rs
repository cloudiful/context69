//! Tracks the static SQLx query files as compiler inputs.
//!
//! `sqlx::query_file*!` reads a `.sql` file while the macro expands, but SQLx
//! wraps the embedded SQL in `include_str!` only for row-returning queries:
//! `query_file_scalar!` and statements with no output columns are expanded from
//! the raw string and leave no dependency edge that Cargo or rustc can see. On
//! top of that, this workspace compiles through `sccache`, whose key covers
//! Rust sources and rustc arguments rather than the `.sql` file, so a query
//! edit can replay a previously compiled crate and keep the old SQL embedded
//! even after `cargo clean`.
//!
//! Declaring every query file as a Cargo rerun trigger and folding its content
//! into the compiler inputs restores the missing edge for both cases.

use std::fs;
use std::path::{Path, PathBuf};

/// Directory holding the `.sql` files loaded through `sqlx::query_file*!`.
const QUERY_DIR: &str = "src/sql";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo"),
    );
    let query_dir = manifest_dir.join(QUERY_DIR);
    // The directory entry catches added and removed query files as well.
    println!("cargo:rerun-if-changed={}", query_dir.display());
    let mut query_files = Vec::new();
    collect_sql_files(&query_dir, &mut query_files);
    query_files.sort();

    let mut stamp = FNV_OFFSET;
    for path in &query_files {
        println!("cargo:rerun-if-changed={}", path.display());
        stamp = mix(stamp, path.to_string_lossy().as_bytes());
        let contents = fs::read(path)
            .unwrap_or_else(|error| panic!("read query file {}: {error}", path.display()));
        stamp = mix(stamp, &contents);
    }
    // The stamp is a compiler input: when a query file changes, the crate's
    // build fingerprint and any content-addressed compile cache key change too.
    println!("cargo:rustc-env=CONTEXT69_SQL_SOURCE_STAMP={stamp:016x}");
}

fn collect_sql_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read query directory {}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("read query directory entry").path();
        if path.is_dir() {
            collect_sql_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "sql") {
            files.push(path);
        }
    }
}

fn mix(state: u64, bytes: &[u8]) -> u64 {
    let mut hash = state;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
