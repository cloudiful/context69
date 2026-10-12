pub mod benchmark;
pub mod endpoint;
mod endpoint_review;
pub mod fixtures;
pub mod oracle;
pub mod parity;

use context69::db::Database;
use uuid::Uuid;

pub fn unique_group_key(label: &str) -> String {
    format!("issue62-{label}-{}", Uuid::new_v4())
}

pub fn assert_owned_database_url(url: &str) {
    if let Err(message) = endpoint::validate(url) {
        panic!("{message}");
    }
}

pub async fn connect_owned_database(url: &str) -> Database {
    assert_owned_database_url(url);
    let bounded_url =
        endpoint::with_statement_timeout(url).unwrap_or_else(|message| panic!("{message}"));
    match Database::connect(&bounded_url).await {
        Ok(database) => database,
        Err(error) => panic!("{}", endpoint::redacted_connection_error(error)),
    }
}
