//! The startup sealing backfill, against a real database.
//!
//! These cases run where the sweep actually runs: a migrated PostgreSQL table
//! holding the legacy plaintext rows the transition state wrote. What they pin
//! down is the contract an operator's key procedure depends on — a keyed startup
//! seals every supported row, each one reopening to exactly the bytes it held;
//! the row's identity and timestamps survive; already-sealed rows are left
//! alone; the work is batched and atomic; a row the catalogue cannot name stops
//! the run without leaking anything; and a competing startup or a concurrent
//! writer never loses or duplicates a conversion.
//!
//! The cases are grouped by what they are about: [`conversion`] for the sweep
//! over the rows it finds, [`refusal`] for the rows it cannot convert, and
//! [`contention`] for what happens when another startup or writer is working on
//! the same rows. Their shared fixtures live beside them in [`backfill_support`]
//! rather than here, so each group reads on its own.
//!
//! Every value here is synthetic. The master keys are fixed test constants (byte
//! ranges `00..20` and `20..40`), the plaintexts are derived from this file's
//! own key markers, and no case names a real key. Cleanup deletes exactly the
//! rows these cases wrote, by marker and by the six fixed singleton names.
//!
//! The sweep is a table-wide operation, so the cases are serialized and each one
//! starts from an empty table; two cases running at once would move each other's
//! rows. They run only when `CONTEXT69_TEST_DATABASE_URL` names a disposable
//! migrated database this binary owns, and print that they were skipped
//! otherwise — a run without it is never reported as coverage. No `.env`,
//! machine configuration, or remote/shared database is used.

mod backfill_support;
