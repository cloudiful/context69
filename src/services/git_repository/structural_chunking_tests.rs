//! Structural chunking tests (issue #28 phase P3).

use std::collections::BTreeSet;

use super::chunk_structural_text;
use crate::services::git_repository::code_text::{
    CodeChunk, CodeChunkBounds, chunk_code_text, chunk_code_text_for_language,
};

fn bounds(max_bytes: usize, max_lines: usize, max_chunks: usize) -> CodeChunkBounds {
    CodeChunkBounds {
        max_bytes,
        max_lines,
        max_chunks,
    }
}

fn assert_exact_and_anchored(source: &str, chunks: &[CodeChunk]) {
    let reassembled = chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    assert_eq!(reassembled, source, "chunks must reassemble byte for byte");
    let mut covered = BTreeSet::new();
    let mut previous_start = 0;
    for (position, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.chunk_index, position as i32);
        assert!(chunk.end_line >= chunk.start_line);
        assert!(chunk.start_line >= previous_start);
        previous_start = chunk.start_line;
        covered.extend(chunk.start_line..=chunk.end_line);
    }
    let expected: BTreeSet<i32> = if source.is_empty() {
        BTreeSet::new()
    } else {
        (1..=source.split_inclusive('\n').count() as i32).collect()
    };
    assert_eq!(covered, expected, "every source line must be named");
}

const RUST_SOURCE: &str = "//! Crate docs.\n\nuse std::fmt;\n\n/// Adds numbers.\n/// Second line.\n#[inline]\npub fn add(left: i32, right: i32) -> i32 {\n    left + right\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {\n        assert_eq!(super::add(1, 2), 3);\n    }\n}\n";

#[test]
fn rust_items_keep_attributes_and_comments_with_their_declaration() {
    let chunks = chunk_structural_text("rust", RUST_SOURCE, CodeChunkBounds::default())
        .expect("structural pass")
        .expect("Rust is structurally supported");

    assert_eq!(chunks.len(), 3);
    assert_exact_and_anchored(RUST_SOURCE, &chunks);

    assert!(
        chunks[0].text.contains("//! Crate docs.") && chunks[0].text.contains("use std::fmt;"),
        "the first chunk keeps the file header with the first item"
    );
    assert!(
        chunks[1].text.starts_with("/// Adds numbers."),
        "the doc comment and attribute open the chunk of the item they document"
    );
    assert!(chunks[1].text.contains("#[inline]"));
    assert!(chunks[1].text.contains("pub fn add"));
    assert_eq!(chunks[1].start_line, 5);
    assert_eq!(chunks[1].end_line, 11);
    assert!(
        chunks[2].text.starts_with("#[cfg(test)]"),
        "the module attribute stays with the module"
    );
    assert!(chunks[2].text.contains("fn adds()"));
}

#[test]
fn typescript_and_tsx_keep_leading_comments_with_declarations() {
    let typescript = "// header\n\nimport { x } from './x';\n\n/** shape */\nexport interface Shape {\n  value: number;\n}\n\nexport function read(shape: Shape) {\n  return shape.value;\n}\n";
    let ts_chunks = chunk_structural_text("typescript", typescript, CodeChunkBounds::default())
        .expect("structural pass")
        .expect("TypeScript is structurally supported");
    assert_eq!(ts_chunks.len(), 3);
    assert_exact_and_anchored(typescript, &ts_chunks);
    assert!(ts_chunks[1].text.starts_with("/** shape */"));
    assert_eq!(ts_chunks[1].start_line, 5);

    let tsx = "// component file\n\nexport function App() {\n  return <main>Hello</main>;\n}\n\nexport const label = 'app';\n";
    let tsx_chunks = chunk_structural_text("tsx", tsx, CodeChunkBounds::default())
        .expect("structural pass")
        .expect("TSX is structurally supported");
    assert_eq!(tsx_chunks.len(), 2);
    assert_exact_and_anchored(tsx, &tsx_chunks);
    assert!(tsx_chunks[0].text.contains("<main>Hello</main>"));
    assert!(tsx_chunks[1].text.contains("export const label"));
}

#[test]
fn oversized_items_are_split_under_the_same_bounds() {
    let mut source = String::from("pub fn huge() {\n");
    for index in 0..120 {
        source.push_str(&format!("    let value_{index} = {index};\n"));
    }
    source.push_str("}\n");
    let limits = bounds(512, 20, 4_096);

    let chunks = chunk_structural_text("rust", &source, limits)
        .expect("structural pass")
        .expect("Rust is structurally supported");
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(chunk.text.len() <= limits.max_bytes, "{}", chunk.text.len());
        assert!(
            (chunk.end_line - chunk.start_line + 1) as usize <= limits.max_lines,
            "chunk lines must stay bounded"
        );
    }
    assert_exact_and_anchored(&source, &chunks);
}

#[test]
fn the_chunk_count_bound_fails_closed_instead_of_truncating() {
    let source = "fn one() {}\n\nfn two() {}\n";
    let error = chunk_structural_text("rust", source, bounds(1_024, 20, 1))
        .expect_err("two structural chunks exceed a one-chunk bound")
        .to_string();
    assert!(error.contains("git_chunk_count_exceeded"), "{error}");
}

#[test]
fn parse_errors_fall_back_to_line_chunking() {
    let broken = "fn broken( {\n    let x = ;\n}\n";
    assert!(
        chunk_structural_text("rust", broken, CodeChunkBounds::default())
            .expect("structural pass")
            .is_none()
    );
    let fallback = chunk_code_text_for_language("rust", broken, CodeChunkBounds::default())
        .expect("fallback chunks");
    assert_eq!(
        fallback,
        chunk_code_text(broken, CodeChunkBounds::default()).unwrap()
    );
}

#[test]
fn unsupported_languages_use_the_explicit_line_fallback() {
    let sql = "CREATE TABLE users (\n  id BIGINT PRIMARY KEY\n);\n";
    assert!(
        chunk_structural_text("sql", sql, CodeChunkBounds::default())
            .expect("structural pass")
            .is_none(),
        "SQL has no pinned grammar and must not claim AST support"
    );
    let fallback = chunk_code_text_for_language("sql", sql, CodeChunkBounds::default())
        .expect("fallback chunks");
    assert_eq!(fallback.len(), 1);
    assert_exact_and_anchored(sql, &fallback);
}

#[test]
fn empty_input_and_comment_only_files_stay_exact() {
    let empty = chunk_structural_text("rust", "", CodeChunkBounds::default())
        .expect("structural pass")
        .expect("empty source still parses");
    assert!(empty.is_empty());
    let comments = "//! only a header\n//! second line\n";
    let chunks = chunk_structural_text("rust", comments, CodeChunkBounds::default())
        .expect("structural pass")
        .expect("Rust is structurally supported");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, comments);
    assert_exact_and_anchored(comments, &chunks);
}
