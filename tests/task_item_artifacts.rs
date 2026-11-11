//! Pure task-item artifact kind units.

use context69::db::TaskItemArtifactKind;

#[test]
fn artifact_kind_pairs_are_fixed_and_unknown_pairs_are_refused() {
    assert_eq!(TaskItemArtifactKind::Download.kind(), "download");
    assert_eq!(TaskItemArtifactKind::Download.encoding(), "raw-v1");
    assert_eq!(TaskItemArtifactKind::Sections.kind(), "sections");
    assert_eq!(
        TaskItemArtifactKind::Sections.encoding(),
        "ingest-sections-json-v1"
    );
    assert_eq!(
        TaskItemArtifactKind::from_parts("download", "raw-v1"),
        Some(TaskItemArtifactKind::Download)
    );
    assert_eq!(
        TaskItemArtifactKind::from_parts("sections", "ingest-sections-json-v1"),
        Some(TaskItemArtifactKind::Sections)
    );
    for (kind, encoding) in [
        ("download", "ingest-sections-json-v1"),
        ("sections", "raw-v1"),
        ("download", "base64"),
        ("unknown", "raw-v1"),
        ("sections", ""),
    ] {
        assert_eq!(
            TaskItemArtifactKind::from_parts(kind, encoding),
            None,
            "unknown pair {kind}/{encoding} must be refused"
        );
    }
}
