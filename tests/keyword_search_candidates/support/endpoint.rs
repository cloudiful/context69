//! Guards the keyword suite's database access against the shared resolver.
//!
//! The suite no longer names its own database and no longer recognises the old
//! per-target runner prefix: the only database it may open is the one the shared
//! resolver selected for this run. Authorization compares the *effective*
//! connection identity, so a query-string override that redirects the host,
//! port, role, or database is refused as well.

use std::str::FromStr;

use context69_test_support::{accessor, config::Endpoint};
use sqlx::postgres::PgConnectOptions;

const INVALID_ENDPOINT: &str = "refuse an invalid PostgreSQL test database URL";
const UNAPPROVED_ENDPOINT: &str = "refuse a PostgreSQL test database the resolver did not select";
const CONNECTION_FAILURE: &str =
    "failed to connect to or initialize the explicitly supplied test database";
const STATEMENT_TIMEOUT: &str = "options=-c%20statement_timeout%3D30000";

/// Everything a query string could use to redirect the connection.
type Identity = (String, u16, String, String);

fn identity(url: &str) -> Result<Identity, &'static str> {
    let options = PgConnectOptions::from_str(url).map_err(|_| INVALID_ENDPOINT)?;
    if options.get_socket().is_some() {
        return Err(UNAPPROVED_ENDPOINT);
    }
    Ok((
        options.get_host().to_owned(),
        options.get_port(),
        options.get_username().to_owned(),
        options.get_database().unwrap_or_default().to_owned(),
    ))
}

fn validated_options(url: &str, selected: &Endpoint) -> Result<PgConnectOptions, &'static str> {
    let candidate = identity(url)?;
    let authorized = selected
        .connect_options()
        .map_err(|_| UNAPPROVED_ENDPOINT)?;
    let selected_identity = (
        authorized.get_host().to_owned(),
        authorized.get_port(),
        authorized.get_username().to_owned(),
        authorized.get_database().unwrap_or_default().to_owned(),
    );
    if candidate != selected_identity {
        return Err(UNAPPROVED_ENDPOINT);
    }
    PgConnectOptions::from_str(url).map_err(|_| INVALID_ENDPOINT)
}

/// Refuse a URL that is not the target the shared resolver selected.
///
/// # Panics
///
/// Panics when no target is configured, which is the case for any keyword suite
/// entry that reaches this point without one.
pub fn validate(url: &str) -> Result<(), &'static str> {
    let selected = Endpoint::parse(&accessor::database_url()).map_err(|_| UNAPPROVED_ENDPOINT)?;
    validate_against(url, &selected)
}

/// The same check against an explicitly supplied selection, so the rule can be
/// exercised without a configured target.
pub fn validate_against(url: &str, selected: &Endpoint) -> Result<(), &'static str> {
    validated_options(url, selected).map(|_| ())
}

pub(super) fn with_statement_timeout(url: &str) -> Result<String, &'static str> {
    if url.contains('#') {
        return Err(UNAPPROVED_ENDPOINT);
    }
    let separator = match url.split_once('?') {
        None => "?",
        Some((_, query)) if query.is_empty() || query.ends_with('&') => "",
        Some(_) => "&",
    };
    Ok(format!("{url}{separator}{STATEMENT_TIMEOUT}"))
}

pub(super) fn redacted_connection_error<E>(_error: E) -> &'static str {
    CONNECTION_FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELECTED: &str = "postgres://keyword:@127.0.0.1:55497/context69";

    fn selected() -> Endpoint {
        Endpoint::parse(SELECTED).expect("the selected URL is well formed")
    }

    #[test]
    fn accepts_only_the_selected_endpoint() {
        assert!(validate_against(SELECTED, &selected()).is_ok());
        assert!(
            validate_against(
                "postgresql://keyword:@127.0.0.1:55497/context69",
                &selected()
            )
            .is_ok()
        );
        for url in [
            "postgres://keyword:@127.0.0.1:55498/context69",
            "postgres://keyword:@127.0.0.2:55497/context69",
            "postgres://other:@127.0.0.1:55497/context69",
            "postgres://keyword:@127.0.0.1:55497/other_database",
        ] {
            assert!(
                validate_against(url, &selected()).is_err(),
                "{url} must not be accepted"
            );
        }
    }

    #[test]
    fn rejects_query_overrides_that_redirect_the_connection() {
        for query in [
            "host=192.0.2.1",
            "hostaddr=192.0.2.1",
            "port=55498",
            "dbname=other_database",
            "user=other",
        ] {
            assert!(
                validate_against(&format!("{SELECTED}?{query}"), &selected()).is_err(),
                "{query} must not be accepted"
            );
        }
    }

    #[test]
    fn preserves_encoded_query_options_and_adds_timeout() {
        let url = "postgresql://keyword:sentinel%40password@127.0.0.1:55497/context69?application_name=keyword%20test&options=-c%20work_mem%3D4MB";
        assert!(validate_against(url, &selected()).is_ok());
        let bounded = with_statement_timeout(url).unwrap();
        assert!(bounded.starts_with(url) && bounded.ends_with(&format!("&{STATEMENT_TIMEOUT}")));
        assert_eq!(bounded.matches('?').count(), 1);
        assert!(PgConnectOptions::from_str(&bounded).is_ok());
        assert!(
            with_statement_timeout("postgres://user:@127.0.0.1:55497/context69?")
                .unwrap()
                .ends_with(&format!("?{STATEMENT_TIMEOUT}"))
        );
    }

    #[test]
    fn rejects_unparseable_and_unresolved_urls() {
        for url in [
            "postgres://@127.0.0.1:55497/context69",
            "postgres://user:@127.0.0.1:55497/",
            "postgres://user:@127.0.0.1:55497/context69#fragment",
            "postgres://user:@127.0.0.1/context69",
            "postgres://user:@/context69",
            "mysql://user@127.0.0.1:55497/context69",
            "not a url",
        ] {
            assert!(
                validate_against(url, &selected()).is_err(),
                "{url} must not be accepted"
            );
        }
    }

    #[test]
    fn validation_and_connection_errors_redact_passwords() {
        let sentinel = "unique-sentinel-password";
        for url in [
            format!("postgres://user:{sentinel}@127.0.0.1:55497/other_database"),
            format!("postgres://user:{sentinel}@%zz:55497/context69"),
        ] {
            let error = validate_against(&url, &selected()).unwrap_err();
            assert!(!error.contains(sentinel), "{error} leaked the password");
        }
        let error = redacted_connection_error(format!("password={sentinel}"));
        assert_eq!(error, CONNECTION_FAILURE);
        assert!(!error.contains(sentinel));
    }
}
