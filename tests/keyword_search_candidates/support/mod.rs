pub mod benchmark;
pub mod endpoint;
#[cfg(test)]
mod endpoint_review;
pub mod fixtures;
pub mod oracle;
pub mod parity;

use context69::db::Database;
use uuid::Uuid;

pub const TEST_DATABASE: &str = "context69_keyword_test";
pub const BENCHMARK_DATABASE: &str = "context69_keyword_perf";
pub const REQUIRED_DATABASE_ENV: &str = "CONTEXT69_KEYWORD_REQUIRE_DATABASE";

pub fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

pub fn unique_group_key(label: &str) -> String {
    format!("issue62-{label}-{}", Uuid::new_v4())
}

pub fn assert_owned_database_url(url: &str, expected_database: &str) {
    if let Err(message) = endpoint::validate(url, expected_database) {
        panic!("{message}");
    }
}

pub async fn connect_owned_database(url: &str, expected_database: &str) -> Database {
    assert_owned_database_url(url, expected_database);
    let bounded_url =
        endpoint::with_statement_timeout(url).unwrap_or_else(|message| panic!("{message}"));
    match Database::connect(&bounded_url).await {
        Ok(database) => database,
        Err(error) => panic!("{}", endpoint::redacted_connection_error(error)),
    }
}
