//! Pure bounded-settings units for the usage recorder.

use std::time::Duration;

use super::last_used_recorder::{FLUSH_INTERVAL, PENDING_CAPACITY, QUEUE_CAPACITY, SINK_TIMEOUT};

#[test]
fn the_recorder_keeps_its_bounded_sizes_and_deadlines() {
    assert_eq!(QUEUE_CAPACITY, 1024);
    assert_eq!(PENDING_CAPACITY, 1024);
    assert_eq!(FLUSH_INTERVAL, Duration::from_secs(60));
    assert_eq!(SINK_TIMEOUT, Duration::from_secs(2));
}
