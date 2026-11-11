//! Pure SQL-contract regressions for the task claim and parent-projection paths.
//!
//! These cases assert the shipped statements as text, so they need no database
//! and stay fast and isolated.

#[path = "task_dispatcher_fast_path/cases_outcome.rs"]
mod cases_outcome;

#[path = "task_dispatcher_fast_path/cases_sql_contract.rs"]
mod cases_sql_contract;

#[path = "task_dispatcher_fast_path/cases_contract_discrimination.rs"]
mod cases_contract_discrimination;
