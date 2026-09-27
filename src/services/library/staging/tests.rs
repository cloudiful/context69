//! Tests for the Docling capacity helpers owned by the enclosing `library`
//! module.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::super::resize_docling_semaphore;

#[test]
fn docling_resize_shrink_fail_then_grow_matches_latest_target() {
    // Regression test for review note 4908: a shrink that cannot
    // reclaim checked-out permits must report the actual total so the
    // next grow deltas from reality instead of over-adding.
    let semaphore = Arc::new(Semaphore::new(4));
    let held: Vec<OwnedSemaphorePermit> = (0..4)
        .map(|_| {
            semaphore
                .clone()
                .try_acquire_owned()
                .expect("initial permit")
        })
        .collect();
    assert_eq!(semaphore.available_permits(), 0);

    let actual = resize_docling_semaphore(&semaphore, 4, 2);
    assert_eq!(
        actual, 4,
        "failed shrink must report the actual total, not the desired target"
    );
    assert_eq!(semaphore.available_permits(), 0);

    let actual = resize_docling_semaphore(&semaphore, actual, 5);
    assert_eq!(actual, 5, "grow must delta from the actual total");
    assert_eq!(
        semaphore.available_permits() + held.len(),
        5,
        "semaphore total must equal the latest target, not over-admit"
    );
    drop(held);
    assert_eq!(semaphore.available_permits(), 5);
}

#[test]
fn docling_resize_shrink_reclaims_idle_permits_exactly() {
    let semaphore = Arc::new(Semaphore::new(4));
    let actual = resize_docling_semaphore(&semaphore, 4, 2);
    assert_eq!(actual, 2);
    assert_eq!(semaphore.available_permits(), 2);
}
