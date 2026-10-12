//! Configuration resolution behaviour of the backend test target.
//!
//! Every fixture is written to a temporary directory with synthetic
//! credentials, so no test in this file can read or print the repository's real
//! `.env` values.

use std::fs;
use std::path::{Path, PathBuf};

use context69_test_support::config::{self, ConfigError, Endpoint, MalformedTarget, TargetSource};

const SYNTHETIC: &str = "postgres://synthetic-user:synthetic-secret@db.test:5433/synthetic_db";

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "context69-test-support-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture directory");
        Self { root }
    }

    fn env_file(&self, body: &str) -> PathBuf {
        let path = self.root.join(".env");
        fs::write(&path, body).expect("fixture .env");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn resolve(override_value: Option<&str>, env_file: &Path) -> Result<TargetSource, ConfigError> {
    config::resolve(override_value, env_file).map(|target| target.source())
}

#[test]
fn explicit_override_wins_over_the_repository_env_file() {
    let fixture = Fixture::new("override-wins");
    let env_file = fixture.env_file(&format!("DATABASE_URL={SYNTHETIC}\n"));

    let target = config::resolve(Some("postgres://other:pw@host:6000/other_db"), &env_file)
        .expect("explicit override is valid");
    assert_eq!(target.source(), TargetSource::EnvironmentOverride);
    assert_eq!(target.endpoint().database(), "other_db");
}

#[test]
fn repository_env_file_is_the_only_fallback() {
    let fixture = Fixture::new("file-fallback");
    let env_file = fixture.env_file(&format!("DATABASE_URL={SYNTHETIC}\n"));

    let target = config::resolve(None, &env_file).expect("file fallback resolves");
    assert_eq!(target.source(), TargetSource::RepositoryEnvFile);
    assert_eq!(target.endpoint().database(), "synthetic_db");
}

#[test]
fn a_missing_target_is_a_failure_and_never_a_pass() {
    let fixture = Fixture::new("missing-target");
    let absent = fixture.root.join("does-not-exist");
    assert_eq!(
        resolve(None, &absent),
        Err(ConfigError::TargetNotConfigured),
        "an absent .env must fail rather than fall back to another endpoint"
    );

    let empty = fixture.env_file("CONTEXT69_APP__MASTER_SECRET=abc\n");
    assert_eq!(
        resolve(None, &empty),
        Err(ConfigError::TargetNotConfigured),
        "a .env without DATABASE_URL must not select a target"
    );
}

#[test]
fn quoted_and_ordered_entries_parse_without_mutating_the_environment() {
    let fixture = Fixture::new("quotes");
    let env_file = fixture.env_file(&format!(
        "OTHER=1\nDATABASE_URL='{SYNTHETIC}'\nDATABASE_URL={SYNTHETIC}\n"
    ));

    let target = config::resolve(None, &env_file).expect("quoted value parses");
    assert_eq!(target.source(), TargetSource::RepositoryEnvFile);
    assert_eq!(target.endpoint().database(), "synthetic_db");
}

#[test]
fn query_options_are_preserved_and_reach_the_connection_options() {
    let endpoint = Endpoint::parse(
        "postgresql://synthetic-user:synthetic-secret@db.test:5433/synthetic_db?sslmode=require",
    )
    .expect("valid target");
    let options = endpoint
        .connect_options()
        .expect("sqlx accepts the selection");
    assert_eq!(options.get_host(), "db.test");
    assert_eq!(options.get_port(), 5433);
    assert_eq!(options.get_database(), Some("synthetic_db"));
    assert_eq!(
        format!("{:?}", options.get_ssl_mode()),
        format!("{:?}", sqlx::postgres::PgSslMode::Require)
    );
}

#[test]
fn endpoint_identity_uses_effective_query_overrides_and_decoded_values() {
    let endpoint = Endpoint::parse(
        "postgres://synthetic_user:synthetic_password@base.test/path_db?host=effective.test&port=5444&user=decoded%20role&dbname=selected_db",
    )
    .expect("valid target");
    let options = endpoint.connect_options().expect("parsed options");

    assert_eq!(endpoint.database(), "selected_db");
    assert_eq!(options.get_host(), "effective.test");
    assert_eq!(options.get_port(), 5444);
    assert_eq!(options.get_username(), "decoded role");
    assert_eq!(options.get_database(), Some("selected_db"));

    let encoded_database =
        Endpoint::parse("postgres://synthetic_user:synthetic_password@db.test/selected%5Fdb")
            .expect("percent-encoded database name");
    assert_eq!(encoded_database.database(), "selected_db");
}

#[test]
fn omitted_port_uses_postgres_default_not_ambient_libpq_configuration() {
    let endpoint = Endpoint::parse("postgres://synthetic_user:synthetic_password@db.test/db")
        .expect("valid target");
    assert_eq!(endpoint.connect_options().unwrap().get_port(), 5432);
}

#[test]
fn child_handoff_matches_normalized_options_with_ambient_pgport() {
    const CHILD: &str = "CONTEXT69_CHILD_HANDOFF_OPTIONS";
    const URL: &str = "postgres://synthetic_user:synthetic_password@base.test/path_db?host=effective.test&user=decoded%20role&dbname=selected_db&sslmode=require&options=-c%20application_name%3Dsynthetic";

    if std::env::var_os(CHILD).is_some() {
        let endpoint = Endpoint::parse(URL).expect("synthetic target");
        let selected = endpoint.connect_options().expect("normalized options");
        let handed_off = endpoint
            .expose()
            .parse::<sqlx::postgres::PgConnectOptions>()
            .expect("normalized child URL");

        assert_eq!(selected.get_host(), handed_off.get_host());
        assert_eq!(selected.get_port(), 5432);
        assert_eq!(selected.get_port(), handed_off.get_port());
        assert_eq!(selected.get_username(), handed_off.get_username());
        assert_eq!(selected.get_database(), handed_off.get_database());
        assert_eq!(
            format!("{:?}", selected.get_ssl_mode()),
            format!("{:?}", handed_off.get_ssl_mode())
        );
        assert_eq!(selected.get_options(), handed_off.get_options());
        return;
    }

    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "child_handoff_matches_normalized_options_with_ambient_pgport",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, "1")
        .env("PGPORT", "6543")
        .status()
        .expect("launch isolated endpoint test");
    assert!(status.success());
}

#[test]
fn malformed_targets_report_a_category_and_never_the_value() {
    let cases = [
        ("mysql://u:p@h/db", MalformedTarget::UnsupportedScheme),
        (
            "postgres://u:p@h/db#frag",
            MalformedTarget::FragmentNotAllowed,
        ),
        ("postgres://u:p@/db", MalformedTarget::MissingHost),
        ("postgres://:p@h/db", MalformedTarget::MissingUsername),
        ("postgres://u:p@h", MalformedTarget::MissingDatabase),
        ("postgres://u:p@h/a/b", MalformedTarget::MissingDatabase),
    ];
    for (raw, expected) in cases {
        let error = Endpoint::parse(raw).expect_err("must be refused");
        assert_eq!(error, expected, "{raw}");
    }

    let empty = Endpoint::parse("   ").expect_err("must be refused");
    assert_eq!(empty, MalformedTarget::UnsupportedScheme);
}

#[test]
fn configuration_errors_carry_no_value_of_any_kind() {
    let fixture = Fixture::new("redaction");
    let secret = "synthetic-secret";
    let env_file = fixture.env_file(&format!(
        "DATABASE_URL=postgres://u:{secret}@h:not-a-port/db\n"
    ));

    let error = config::resolve(None, &env_file).expect_err("invalid port is refused");
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(secret), "{rendered}");
    assert!(!rendered.contains("db.test"), "{rendered}");
    assert_eq!(
        error,
        ConfigError::EnvFileValueMalformed(MalformedTarget::MissingHost)
    );
}

#[test]
fn an_unreadable_or_malformed_env_file_is_categorized_without_its_lines() {
    let fixture = Fixture::new("malformed-file");
    let env_file = fixture.env_file("DATABASE_URL=\"unterminated\n");
    let error = config::resolve(None, &env_file).expect_err("unterminated quote is refused");
    assert!(matches!(
        error,
        ConfigError::EnvFileMalformed | ConfigError::EnvFileValueMalformed(_)
    ));
    assert!(!format!("{error:?} {error}").contains("unterminated"));
}

#[test]
fn an_empty_override_is_refused_rather_than_falling_back() {
    let fixture = Fixture::new("empty-override");
    let env_file = fixture.env_file(&format!("DATABASE_URL={SYNTHETIC}\n"));
    assert_eq!(
        resolve(Some("   "), &env_file),
        Err(ConfigError::OverrideEmpty),
        "an empty override must not silently select the file value"
    );
}

#[test]
fn endpoint_rendering_and_fingerprints_expose_no_connection_detail() {
    let endpoint = Endpoint::parse(SYNTHETIC).expect("valid target");
    let rendered = format!("{endpoint:?} {endpoint}");
    for secret in [
        "synthetic-secret",
        "db.test",
        "synthetic_db",
        "synthetic-user",
    ] {
        assert!(!rendered.contains(secret), "{rendered} leaked {secret}");
    }
    assert_eq!(endpoint.fingerprint().len(), 16);

    let same = Endpoint::parse(SYNTHETIC).expect("valid target");
    assert_eq!(endpoint.fingerprint(), same.fingerprint());
    let other = Endpoint::parse("postgres://synthetic-user:synthetic-secret@db.test:5433/other_db")
        .expect("valid target");
    assert_ne!(endpoint.fingerprint(), other.fingerprint());
}
