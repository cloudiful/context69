#[path = "keyword_search_candidates/support/mod.rs"]
mod support;

#[tokio::test]
#[ignore = "requires configured PostgreSQL and CONTEXT69_KEYWORD_BENCHMARK"]
async fn keyword_search_matches_frozen_head_oracle() {
    let database_url = context69_test_support::accessor::database_url();
    support::parity::run(&database_url).await;
}

#[tokio::test]
#[ignore = "requires configured PostgreSQL and CONTEXT69_KEYWORD_BENCHMARK"]
async fn keyword_search_benchmark_baseline() {
    let database_url = context69_test_support::accessor::database_url();
    support::benchmark::run(&database_url).await;
}
