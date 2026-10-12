use std::str::FromStr;

use context69_test_support::config::Endpoint;
use sqlx::postgres::PgConnectOptions;

use super::endpoint;

fn selected(url: &str) -> Endpoint {
    Endpoint::parse(url).expect("the selected URL is well formed")
}

#[test]
fn authorization_follows_the_resolver_rather_than_a_named_database() {
    // Any endpoint the resolver selects is authorized; nothing about a
    // particular port or database name is special.
    for url in [
        "postgresql://keyword:@localhost:55497/context69",
        "postgresql://keyword:@localhost:6000/context69",
        "postgresql://keyword:@db.example:65432/context69_keyword_test",
    ] {
        assert!(
            endpoint::validate_against(url, &selected(url)).is_ok(),
            "{url} is authorized once the resolver selects it"
        );
    }
    let chosen = "postgresql://keyword:@localhost:55497/context69";
    assert!(
        endpoint::validate_against(
            "postgresql://keyword:@localhost:55497/context69_keyword_test",
            &selected(chosen)
        )
        .is_err(),
        "a database the resolver did not select stays refused"
    );
}

#[test]
fn bounded_options_are_still_authorized_and_keep_their_settings() {
    let selected_url = "postgresql://keyword:@localhost:55497/context69";
    let url = format!(
        "{selected_url}?application_name=review-pure\
         &options=-c%20work_mem%3D4MB%20-c%20statement_timeout%3D0"
    );
    assert!(endpoint::validate_against(&url, &selected(selected_url)).is_ok());
    let bounded = endpoint::with_statement_timeout(&url).unwrap();
    assert!(endpoint::validate_against(&bounded, &selected(selected_url)).is_ok());
    let parsed = PgConnectOptions::from_str(&bounded).unwrap();
    assert_eq!(parsed.get_application_name(), Some("review-pure"));
    assert_eq!(
        parsed.get_options(),
        Some("-c work_mem=4MB -c statement_timeout=0 -c statement_timeout=30000")
    );
}
