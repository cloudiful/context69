//! Exact code text validation and line-aware chunking (issue #681 work unit
//! 3B2).

use std::collections::BTreeSet;

use super::{CodeChunk, CodeChunkBounds, CodeText, CodeTextRejection, chunk_code_text};
use crate::db::MAX_GIT_FILE_BYTES;

/// The ceiling `chk_git_generation_chunks_text` enforces on stored chunk text.
/// A chunker bound above it would let a chunk exist that the database refuses.
const STORED_CHUNK_CEILING: usize = 16 * 1024;

fn bounds(max_bytes: usize, max_lines: usize, max_chunks: usize) -> CodeChunkBounds {
    CodeChunkBounds {
        max_bytes,
        max_lines,
        max_chunks,
    }
}

fn validate(text: &str) -> CodeText {
    CodeText::validate(text.as_bytes()).expect("text must be storable")
}

/// Every invariant a caller may rely on: exact reassembly, chunk positions that
/// are zero-based and contiguous, line ranges that never run backwards, and every
/// source line named by at least one chunk.
///
/// A line cut at a character boundary produces several chunks that share one
/// line number, so the ranges are in source order and complete rather than
/// strictly contiguous.
fn assert_exact_and_anchored(source: &str, chunks: &[CodeChunk]) {
    let reassembled = chunks
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect::<String>();
    assert_eq!(
        reassembled, source,
        "concatenating the chunks must reproduce the file byte for byte"
    );
    let mut covered = BTreeSet::new();
    let mut previous_start = 0;
    for (position, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.chunk_index, position as i32);
        assert!(chunk.end_line >= chunk.start_line);
        assert!(
            chunk.start_line >= previous_start,
            "chunks stay in source order"
        );
        previous_start = chunk.start_line;
        covered.extend(chunk.start_line..=chunk.end_line);
    }
    let code_text = validate(source);
    let expected = (1..=code_text.line_count() as i32).collect::<BTreeSet<i32>>();
    assert_eq!(
        covered, expected,
        "every source line is named by at least one chunk"
    );
}

/// Every emitted chunk is inside both the caller's bound and the stored ceiling.
fn assert_within_bound(chunks: &[CodeChunk], max_bytes: usize) {
    for chunk in chunks {
        assert!(
            chunk.text.len() <= max_bytes,
            "a chunk of {} bytes exceeds the {max_bytes} byte bound",
            chunk.text.len()
        );
        assert!(
            chunk.text.len() <= STORED_CHUNK_CEILING,
            "a chunk of {} bytes cannot be stored",
            chunk.text.len()
        );
    }
}

#[test]
fn accepted_text_reports_its_exact_size_and_line_count() {
    let file = validate("fn main() {}\n");
    assert_eq!(file.as_str(), "fn main() {}\n");
    assert_eq!(
        file.byte_count(),
        13,
        "the raw byte length, terminator included"
    );
    assert_eq!(file.line_count(), 1);

    // A missing trailing newline does not invent a line, and a trailing newline
    // does not add an empty one.
    assert_eq!(validate("a\nb").line_count(), 2);
    assert_eq!(validate("a\nb\n").line_count(), 2);
    assert_eq!(validate("\n").line_count(), 1);
    let empty = validate("");
    assert_eq!(empty.line_count(), 0);
    assert_eq!(empty.byte_count(), 0);
    assert!(
        chunk_code_text("", CodeChunkBounds::default())
            .expect("empty text chunks cleanly")
            .is_empty()
    );
}

#[test]
fn unstoreable_bytes_are_rejected_before_any_chunking() {
    assert_eq!(
        CodeText::validate(&[0xFF, 0xFE, 0x00]).expect_err("invalid utf-8 is refused"),
        CodeTextRejection::NotUtf8
    );
    assert_eq!(
        CodeText::validate(b"let x = 1;\0").expect_err("a NUL is refused"),
        CodeTextRejection::ContainsNul
    );
    let oversized = vec![b'a'; MAX_GIT_FILE_BYTES + 1];
    assert_eq!(
        CodeText::validate(&oversized).expect_err("an oversized file is refused"),
        CodeTextRejection::TooLarge
    );
    assert_eq!(
        CodeTextRejection::NotUtf8.error_code(),
        "git_code_text_not_utf8",
        "each rejection maps to its own domain code"
    );
    // A file exactly at the ceiling is still accepted.
    let at_ceiling = vec![b'a'; MAX_GIT_FILE_BYTES];
    assert_eq!(
        CodeText::validate(&at_ceiling)
            .expect("a file at the ceiling is accepted")
            .byte_count(),
        MAX_GIT_FILE_BYTES as i64
    );
}

#[test]
fn chunks_preserve_every_byte_including_whitespace_and_line_endings() {
    for source in [
        "fn main() {\n    println!(\"hi\");\n}\n",
        "a\r\nb\r\n",
        "trailing spaces   \nand tabs\t\n",
        "no trailing newline",
        "unicode ünïcode ✅ 漢字\n",
        "\n\n\n",
        "one",
    ] {
        let code_text = validate(source);
        let chunks = chunk_code_text(code_text.as_str(), CodeChunkBounds::default())
            .expect("chunking succeeds");
        assert_exact_and_anchored(source, &chunks);
        for chunk in &chunks {
            assert!(!chunk.text.contains('\0'), "chunk text must stay NUL-free");
        }
    }
}

#[test]
fn line_bounds_split_whole_lines_and_keep_ranges_inclusive() {
    let source = "1\n2\n3\n4\n5\n";
    let chunks = chunk_code_text(source, bounds(4096, 2, 100)).expect("chunking succeeds");
    assert_eq!(chunks.len(), 3);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| (chunk.start_line, chunk.end_line, chunk.text.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, 2, "1\n2\n"), (3, 4, "3\n4\n"), (5, 5, "5\n")],
        "chunks cover inclusive 1-based line ranges and hold whole lines only"
    );
    assert_exact_and_anchored(source, &chunks);
}

#[test]
fn a_line_that_fits_alone_stays_whole_in_its_own_chunk() {
    let long_line = format!("{}\n", "x".repeat(50));
    let source = format!("short\n{long_line}short\n");
    let chunks = chunk_code_text(&source, bounds(52, 100, 100)).expect("chunking succeeds");
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>(),
        vec!["short\n", long_line.as_str(), "short\n"],
        "a line that fits a chunk on its own is never cut"
    );
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| (chunk.start_line, chunk.end_line))
            .collect::<Vec<_>>(),
        vec![(1, 1), (2, 2), (3, 3)]
    );
    assert_exact_and_anchored(&source, &chunks);
    assert_within_bound(&chunks, 52);
}

#[test]
fn an_overlong_ascii_line_is_cut_into_pieces_that_share_its_line_range() {
    let source = format!("{}\nshort\n", "x".repeat(250));
    let chunks = chunk_code_text(&source, bounds(100, 100, 100)).expect("chunking succeeds");
    assert_within_bound(&chunks, 100);
    assert_exact_and_anchored(&source, &chunks);
    let long_line_pieces = &chunks[..3];
    assert_eq!(
        long_line_pieces
            .iter()
            .map(|piece| piece.text.len())
            .collect::<Vec<_>>(),
        vec![100, 100, 51],
        "the 251 byte line is cut at the last byte boundary inside the bound"
    );
    for piece in long_line_pieces {
        assert_eq!(
            (piece.start_line, piece.end_line),
            (1, 1),
            "every piece keeps the line it was cut from"
        );
        assert!(piece.text.chars().all(|c| c == 'x' || c == '\n'));
    }
    assert_eq!(chunks[3].text, "short\n");
    assert_eq!(
        chunks[3].start_line, 2,
        "the next line starts its own chunk"
    );
}

#[test]
fn an_overlong_multibyte_line_is_cut_only_at_character_boundaries() {
    // Four-byte characters, so a naive byte cut would split them and produce
    // chunks that are not valid UTF-8 text at all.
    let source = format!("{}\n", "漢".repeat(200));
    let pieces = |max_bytes| {
        chunk_code_text(&source, bounds(max_bytes, 100, 100)).expect("chunking succeeds")
    };
    let chunks = pieces(100);
    assert_within_bound(&chunks, 100);
    assert_exact_and_anchored(&source, &chunks);
    // 600 bytes of three-byte characters, then the terminator: 33 whole
    // characters (99 bytes) per piece, never half a character.
    assert_eq!(
        chunks
            .iter()
            .map(|piece| piece.text.len())
            .collect::<Vec<_>>(),
        vec![99, 99, 99, 99, 99, 99, 7]
    );
    for piece in &chunks {
        assert_eq!(
            (piece.start_line, piece.end_line),
            (1, 1),
            "every piece keeps the single line it was cut from"
        );
    }
    // A bound one byte past a character boundary cuts at the same place: the cut
    // follows the characters, not the bytes.
    assert_eq!(
        pieces(101)
            .iter()
            .map(|piece| piece.text.len())
            .collect::<Vec<_>>(),
        vec![99, 99, 99, 99, 99, 99, 7]
    );
    // A bound that cannot hold a whole character keeps the character whole
    // rather than emitting text cut mid character.
    let tiny = chunk_code_text(&source, bounds(2, 100, 1_000)).expect("chunking succeeds");
    assert_exact_and_anchored(&source, &tiny);
    assert_eq!(tiny[0].text, "漢");
}

#[test]
fn an_overlong_line_does_not_push_its_pieces_past_the_line_bound() {
    // The pieces of one line each span a single line, so cutting a long line
    // never counts as extra lines in a later chunk.
    let source = format!("{}\n{}\n", "x".repeat(150), "y".repeat(10));
    let chunks = chunk_code_text(&source, bounds(100, 1, 100)).expect("chunking succeeds");
    assert_within_bound(&chunks, 100);
    assert_exact_and_anchored(&source, &chunks);
    for piece in &chunks {
        assert_eq!(
            (piece.start_line, piece.end_line),
            (piece.start_line, piece.start_line)
        );
    }
}

#[test]
fn chunk_count_bound_is_enforced_instead_of_silently_truncating() {
    let source = "1\n2\n3\n4\n5\n";
    assert!(
        chunk_code_text(source, bounds(4096, 2, 2)).is_err(),
        "a file needing more chunks than the caller bounded is refused rather \
         than losing its tail"
    );
    assert!(
        chunk_code_text(source, bounds(4096, 2, 3)).is_ok(),
        "exactly the bounded number of chunks is still accepted"
    );
    // Cutting one long line into pieces counts against the same bound.
    assert!(
        chunk_code_text(&"x".repeat(500), bounds(100, 100, 4)).is_err(),
        "a long line cannot buy extra rows past the chunk bound"
    );
}

#[test]
fn default_bounds_keep_every_chunk_storable() {
    let default_bounds = CodeChunkBounds::default();
    assert!(
        default_bounds.max_bytes <= STORED_CHUNK_CEILING,
        "the default bound must stay under the stored ceiling"
    );
    // Ordinary lines, a minified one-line file, and multibyte text together: the
    // stored ceiling holds for every chunk, not only for well-behaved input.
    let ordinary = (0..1_000)
        .map(|line| format!("line {line} with some text to fill the chunk\n"))
        .collect::<String>();
    let minified = format!("{}\n", "a,".repeat(200_000));
    let multibyte = format!("{}\n", "✅漢".repeat(100_000));
    for source in [ordinary, minified, multibyte] {
        let chunks = chunk_code_text(&source, default_bounds).expect("chunking succeeds");
        assert_exact_and_anchored(&source, &chunks);
        assert_within_bound(&chunks, default_bounds.max_bytes);
        for chunk in &chunks {
            assert!(
                (chunk.end_line - chunk.start_line) as usize + 1 <= default_bounds.max_lines,
                "a chunk never spans more lines than the bound allows"
            );
        }
    }
}
