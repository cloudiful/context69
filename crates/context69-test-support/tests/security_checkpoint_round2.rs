use context69_test_support::config::Endpoint;
use std::process::Command;

#[test]
fn ambient_pgport_cannot_redirect_an_explicit_url() {
    const CHILD: &str = "CONTEXT69_REVIEW_PORT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let endpoint =
            Endpoint::parse("postgres://synthetic_user:synthetic_password@db.test/synthetic_db")
                .unwrap();
        let options = endpoint.connect_options().unwrap();
        assert_eq!(options.get_port(), 5432);
        return;
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ambient_pgport_cannot_redirect_an_explicit_url",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, "1")
        .env("PGPORT", "6543")
        .status()
        .unwrap();
    assert!(status.success());
}
