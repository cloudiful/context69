use std::{net::IpAddr, str::FromStr};

use sqlx::postgres::PgConnectOptions;

use super::{BENCHMARK_DATABASE, TEST_DATABASE};

const INVALID_ENDPOINT: &str = "refuse an invalid or unapproved PostgreSQL test database URL";
const UNSAFE_ENDPOINT: &str = "refuse an unapproved PostgreSQL test database endpoint";
const CONNECTION_FAILURE: &str =
    "failed to connect to or initialize the explicitly supplied test database";
const STATEMENT_TIMEOUT: &str = "options=-c%20statement_timeout%3D30000";

fn explicit_url_fields(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("postgres") && !scheme.eq_ignore_ascii_case("postgresql")
        || url.contains('#')
    {
        return false;
    }

    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some((userinfo, host_port)) = authority.rsplit_once('@') else {
        return false;
    };
    if userinfo.split(':').next().is_none_or(str::is_empty) {
        return false;
    }

    let port = if let Some(bracketed_host) = host_port.strip_prefix('[') {
        let Some((host, suffix)) = bracketed_host.split_once(']') else {
            return false;
        };
        if host.is_empty() {
            return false;
        }
        suffix.strip_prefix(':')
    } else {
        let Some((host, port)) = host_port.rsplit_once(':') else {
            return false;
        };
        if host.is_empty() {
            return false;
        }
        Some(port)
    };
    if port.is_none_or(str::is_empty) {
        return false;
    }

    let path = rest[authority_end..].split('?').next().unwrap_or_default();
    !path.trim_start_matches('/').is_empty()
}

fn validated_options(url: &str, expected_database: &str) -> Result<PgConnectOptions, &'static str> {
    if !explicit_url_fields(url) {
        return Err(UNSAFE_ENDPOINT);
    }
    let options = PgConnectOptions::from_str(url).map_err(|_| INVALID_ENDPOINT)?;
    if options.get_socket().is_some()
        || !is_loopback_host(options.get_host())
        || options.get_port() == 5432
        || options.get_username().is_empty()
    {
        return Err(UNSAFE_ENDPOINT);
    }

    let database = options
        .get_database()
        .filter(|database| !database.is_empty())
        .ok_or(UNSAFE_ENDPOINT)?;
    if !database_is_allowed(database, expected_database) {
        return Err(UNSAFE_ENDPOINT);
    }
    Ok(options)
}

fn is_loopback_host(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn database_is_allowed(database: &str, expected_database: &str) -> bool {
    match expected_database {
        TEST_DATABASE => database == TEST_DATABASE || is_runner_database(database),
        BENCHMARK_DATABASE => database == BENCHMARK_DATABASE,
        _ => false,
    }
}

fn is_runner_database(database: &str) -> bool {
    let Some(suffix) = database.strip_prefix("context69_test_backend_") else {
        return false;
    };
    let Some((run_id, index)) = suffix.split_once('_') else {
        return false;
    };
    run_id.len() == 32
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && index.len() >= 3
        && index.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn validate(url: &str, expected_database: &str) -> Result<(), &'static str> {
    validated_options(url, expected_database).map(|_| ())
}

pub(super) fn with_statement_timeout(url: &str) -> Result<String, &'static str> {
    if url.contains('#') {
        return Err(UNSAFE_ENDPOINT);
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

    #[test]
    fn accepts_direct_and_runner_owned_databases() {
        for (url, database) in [
            (
                "postgres://user:@127.0.0.1:55497/context69_keyword_test",
                TEST_DATABASE,
            ),
            (
                "postgres://user:@127.0.0.1:55497/context69_test_backend_0123456789abcdef0123456789abcdef_001",
                TEST_DATABASE,
            ),
            (
                "postgres://user:@[::1]:55497/context69_keyword_test",
                TEST_DATABASE,
            ),
            (
                "postgres://user:@localhost:55497/context69_keyword_perf",
                BENCHMARK_DATABASE,
            ),
        ] {
            assert!(validate(url, database).is_ok());
        }
    }

    #[test]
    fn preserves_encoded_query_options_and_adds_timeout() {
        let url = "postgresql://context69%5Ftest:sentinel%40password@localhost:55497/context69%5Fkeyword_test?application_name=keyword%20test&options=-c%20work_mem%3D4MB";
        assert!(validate(url, TEST_DATABASE).is_ok());
        let bounded = with_statement_timeout(url).unwrap();
        assert!(bounded.starts_with(url) && bounded.ends_with(&format!("&{STATEMENT_TIMEOUT}")));
        assert_eq!(bounded.matches('?').count(), 1);
        assert!(PgConnectOptions::from_str(&bounded).is_ok());
        assert!(
            with_statement_timeout("postgres://user:@localhost:55497/db?")
                .unwrap()
                .ends_with(&format!("?{STATEMENT_TIMEOUT}"))
        );
    }

    #[test]
    fn rejects_unsafe_targets_and_runner_names() {
        for url in [
            "postgres://user:@127.0.0.1:5432/context69_keyword_test",
            "postgres://user:@192.0.2.1:55497/context69_keyword_test",
            "postgres://user:@db.example:55497/context69_keyword_test",
            "postgres://@127.0.0.1:55497/context69_keyword_test",
            "postgres://user:@127.0.0.1:55497/",
            "postgres://user:@127.0.0.1:55497/?dbname=",
            "postgres://user:@127.0.0.1:55497/context69_keyword_test#fragment",
            "postgres://user:@127.0.0.1/context69_keyword_test",
            "postgres://user:@/context69_keyword_test",
            "mysql://user@127.0.0.1:55497/context69_keyword_test",
            "postgres://user:@127.0.0.1:55497/other_database",
            "postgres://user:@127.0.0.1:55497/context69_test_backend_0123456789abcdef0123456789abcde_001",
            "postgres://user:@127.0.0.1:55497/context69_test_backend_0123456789ABCDEF0123456789ABCDEF_001",
            "postgres://user:@127.0.0.1:55497/context69_test_backend_0123456789abcdef0123456789abcdef_01",
            "postgres://user:@127.0.0.1:55497/context69_test_backend_0123456789abcdef0123456789abcdef_001x",
        ] {
            assert!(validate(url, TEST_DATABASE).is_err());
        }
        assert!(validate(
            "postgres://user:@127.0.0.1:55497/context69_test_backend_0123456789abcdef0123456789abcdef_001",
            BENCHMARK_DATABASE
        ).is_err());
        assert!(
            validate(
                "postgres://user:@127.0.0.1:55497/context69_keyword_test",
                "unexpected_database"
            )
            .is_err()
        );
    }

    #[test]
    fn validation_and_connection_errors_redact_passwords() {
        let sentinel = "unique-sentinel-password";
        for url in [
            format!("postgres://user:{sentinel}@192.0.2.1:55497/other_database"),
            format!("postgres://user:{sentinel}@%zz:55497/context69_keyword_test"),
        ] {
            assert!(
                !validate(&url, TEST_DATABASE)
                    .unwrap_err()
                    .contains(sentinel)
            );
        }
        let error = redacted_connection_error(format!("password={sentinel}"));
        assert_eq!(error, CONNECTION_FAILURE);
        assert!(!error.contains(sentinel));
    }
}
