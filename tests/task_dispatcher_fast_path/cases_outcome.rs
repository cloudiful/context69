use context69::db::ClaimMaintenanceOutcome;

#[tokio::test]
async fn maintain_outcome_default_is_zero_when_no_rows_changed() {
    let outcome = ClaimMaintenanceOutcome::default();
    assert_eq!(outcome.exhausted_items, 0);
    assert_eq!(outcome.exhausted_files, 0);
    assert_eq!(outcome.exhausted_tasks, 0);
    assert_eq!(outcome.expired_attempts, 0);
}
