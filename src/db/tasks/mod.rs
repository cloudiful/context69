//! Task persistence split by responsibility: row/request types, read queries,
//! submission, dispatcher claiming, item lifecycle transitions, and
//! task-level administration. The public surface is re-exported through
//! `db::mod`.

mod admin;
mod claims;
mod docling_remote_finalize;
mod docling_remote_jobs;
mod item_lifecycle;
mod queries;
mod submission;
mod types;

pub use docling_remote_finalize::{DoclingFailureFinish, DoclingRemoteJobCounts};
pub use docling_remote_jobs::StoredDoclingRemoteJob;
pub use types::{
    ClaimMaintenanceOutcome, ClaimedItem, CreateTaskSubmissionRequest, FinishTaskItemRequest,
    InsertTaskItemRequest, StoredTask, StoredTaskItem, TaskCountFilter, TaskListFilter,
    WaitTaskItemRequest,
};

/// Every item of the collapsed pipeline (issue 529 Task 4) is created in the
/// single `processing` stage. The worker runs the whole pipeline inside one
/// claim and never advances the column, so no other value is written.
const INITIAL_ITEM_STAGE: &str = "processing";
