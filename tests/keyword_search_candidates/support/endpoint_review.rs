use super::{BENCHMARK_DATABASE, TEST_DATABASE, endpoint};
use sqlx::postgres::PgConnectOptions;
use std::str::FromStr;

#[test]
fn endpoint_accepts_actual_runner_name_format_without_connecting() {
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    for index in [1, 999, 1000] {
        let database = format!("context69_test_backend_{run_id}_{index:03}");
        let url = format!("postgresql://user:@localhost:55497/{database}");
        assert!(endpoint::validate(&url, TEST_DATABASE).is_ok());
        assert!(endpoint::validate(&url, BENCHMARK_DATABASE).is_err());
    }
}

#[test]
fn endpoint_checks_effective_query_overrides_and_bounded_options() {
    let base = "postgres://user:@127.0.0.1:55497/context69_keyword_test";
    for query in [
        "host=192.0.2.1",
        "hostaddr=192.0.2.1",
        "host=%2Fsynthetic-socket",
        "port=5432",
        "dbname=postgres",
        "user=",
    ] {
        assert!(endpoint::validate(&format!("{base}?{query}"), TEST_DATABASE).is_err());
    }
    let url = format!(
        "{base}?application_name=review-pure&options=-c%20work_mem%3D4MB%20-c%20statement_timeout%3D0"
    );
    assert!(endpoint::validate(&url, TEST_DATABASE).is_ok());
    let bounded = endpoint::with_statement_timeout(&url).unwrap();
    assert!(endpoint::validate(&bounded, TEST_DATABASE).is_ok());
    let parsed = PgConnectOptions::from_str(&bounded).unwrap();
    assert_eq!(parsed.get_application_name(), Some("review-pure"));
    assert_eq!(
        parsed.get_options(),
        Some("-c work_mem=4MB -c statement_timeout=0 -c statement_timeout=30000")
    );
}
