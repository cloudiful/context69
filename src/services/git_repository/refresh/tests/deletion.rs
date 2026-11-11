//! Pure repository-key payload units.

use serde_json::json;
use uuid::Uuid;

#[test]
fn a_repository_key_is_declared_only_by_a_single_well_formed_payload() {
    use crate::db::task_git_dedup::declared_git_repository_key;

    let key = Uuid::new_v4();
    assert_eq!(
        declared_git_repository_key(&[json!({ "repository_key": key.to_string() })]),
        Some(key)
    );
    assert!(declared_git_repository_key(&[]).is_none());
    assert!(
        declared_git_repository_key(&[json!({ "repository_key": key.to_string() }), json!({})])
            .is_none(),
        "a task names one repository or none"
    );
    assert!(declared_git_repository_key(&[json!({ "repository_key": "not-a-uuid" })]).is_none());
}
