#[path = "keyword_search_candidates/support/mod.rs"]
mod support;

#[tokio::test]
async fn keyword_search_matches_frozen_head_oracle() {
    let Some(database_url) = support::test_database_url() else {
        if std::env::var(support::REQUIRED_DATABASE_ENV).as_deref() == Ok("1") {
            panic!("CONTEXT69_TEST_DATABASE_URL is required for keyword candidate parity");
        }
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping keyword candidate parity");
        return;
    };
    support::parity::run(&database_url).await;
}

#[tokio::test]
async fn keyword_search_benchmark_baseline() {
    if std::env::var("CONTEXT69_KEYWORD_BENCHMARK").as_deref() != Ok("1") {
        return;
    }
    let database_url = std::env::var("CONTEXT69_KEYWORD_BENCHMARK_DATABASE_URL")
        .expect("CONTEXT69_KEYWORD_BENCHMARK_DATABASE_URL is required when benchmark is enabled");
    support::benchmark::run(&database_url).await;
}
