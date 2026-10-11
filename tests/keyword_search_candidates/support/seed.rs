#[path = "seed_rows.rs"]
mod rows;
#[path = "seed_aux.rs"]
mod seed_aux;
#[path = "seed_base.rs"]
mod seed_base;

use context69::db::Database;
use uuid::Uuid;

use super::super::unique_group_key;
use super::SeededFixture;
use rows::group;

pub async fn seed(db: &Database) -> SeededFixture {
    let tag = Uuid::new_v4().simple().to_string();
    let group_key = unique_group_key("public");
    let group_path = format!("issue62/{tag}/public");
    let group_id = group(
        db,
        &group_key,
        "Issue 62 synthetic public group",
        "public",
        &group_path,
    )
    .await;
    let private_group_key = unique_group_key("private");
    let private_group_path = format!("issue62/{tag}/private");
    let private_group_id = group(
        db,
        &private_group_key,
        "Issue 62 synthetic private group",
        "private",
        &private_group_path,
    )
    .await;

    seed_base::seed(db, group_id, private_group_id, &tag).await;
    seed_aux::seed(db, group_id, &tag).await;
    seed_aux::seed_private_translation(db, private_group_id, &tag).await;

    SeededFixture {
        tag,
        group_key,
        group_path,
        private_group_id,
        private_group_key,
        private_group_path,
    }
}
