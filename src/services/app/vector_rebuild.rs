//! Explicit-only vector rebuild.
//!
//! Startup owns no rebuild path: a persisted identity mismatch leaves the
//! runtime unready and schedules nothing, and no path deletes or recreates an
//! existing collection. Re-embedding happens only through the explicit manual
//! rebuild in `services::sync`, which opens the shared rebuild fence before it
//! starts and records the identity it actually re-embedded with.
