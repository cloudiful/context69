//! The embedding/qdrant dependency gate state.
//!
//! Kept apart from the gate call sites so the rule that decides whether the
//! gates open is one small, directly testable unit: a deployment that has the
//! runtime configured but is mid-rebuild is held closed on purpose.

/// Resolves the embedding/qdrant gate state.
///
/// `configured` is the deployment's runtime configuration; `ready` is the live
/// vector-index readiness. A configured runtime that is mid-rebuild is held
/// closed, so writers wait instead of adding vectors with one identity to a
/// collection being re-embedded with another.
pub(super) fn embedding_gate_configuration(
    configured: bool,
    ready: bool,
) -> (bool, Option<&'static str>) {
    if !configured {
        return (
            false,
            Some("configuration: embedding/vector runtime is not configured"),
        );
    }
    if !ready {
        return (false, Some("configuration: vector index is rebuilding"));
    }
    (true, None)
}

#[cfg(test)]
mod tests {
    use super::embedding_gate_configuration;

    #[test]
    fn an_unready_index_holds_the_embedding_gates_closed() {
        let (configured, message) = embedding_gate_configuration(true, false);
        assert!(!configured);
        assert_eq!(message, Some("configuration: vector index is rebuilding"));

        // Ready and configured is the only open state ...
        assert_eq!(embedding_gate_configuration(true, true), (true, None));
        // ... and an unconfigured runtime stays closed regardless of readiness.
        let (configured, message) = embedding_gate_configuration(false, true);
        assert!(!configured);
        assert_eq!(
            message,
            Some("configuration: embedding/vector runtime is not configured")
        );
    }
}
