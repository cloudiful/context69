//! Pure line-window, page, and continuation tests for the bounded content read
//! (issue #681 phase 5C).
//!
//! These exercise the window contract itself — the bounds a caller may name, the
//! verbatim text a page assembles, the byte cap, the row bound, and the
//! server-issued continuation — over stored chunk rows built in memory. No
//! database, network, or secret is involved, and the route's own composition
//! functions are the ones under test.

use axum::http::StatusCode;

use crate::contracts::sources::{MAX_GIT_CONTENT_WINDOW_BYTES, MAX_GIT_CONTENT_WINDOW_LINES};
use crate::db::StoredGitGenerationChunk;

use super::tests::{chunk, file, response_body};
use super::{MAX_CHUNK_PAGE_ROWS, content_page, decode_cursor};

#[test]
fn a_line_window_is_bounded_positive_and_ordered() {
    let entry = file();
    // The accepted span is the inclusive one the caller named.
    for (start_line, end_line) in [(1, 1), (1, 400), (900, 1_100), (i32::MAX - 1, i32::MAX)] {
        let window = entry
            .line_window(start_line, end_line, MAX_GIT_CONTENT_WINDOW_LINES)
            .unwrap_or_else(|error| panic!("{start_line}..={end_line} must be accepted: {error}"));
        assert_eq!(window.start_line, start_line);
        assert_eq!(window.end_line, end_line);
    }
    // Zero, negative, reversed, and over-wide spans are bounded invalid
    // arguments, never a silently narrowed window.
    for (start_line, end_line) in [
        (0, 10),
        (10, 0),
        (-5, 10),
        (10, -5),
        (-1, -1),
        (11, 10),
        (1, MAX_GIT_CONTENT_WINDOW_LINES as i32 + 1),
        (1, i32::MAX),
        (i32::MIN, i32::MAX),
    ] {
        let error = entry
            .line_window(start_line, end_line, MAX_GIT_CONTENT_WINDOW_LINES)
            .expect_err(&format!("{start_line}..={end_line} must be refused"))
            .to_string();
        assert!(
            error.contains("git_line_window"),
            "{start_line}..={end_line} must report a bounded window error: {error}"
        );
    }
    // A window reaching past the entry's last line is accepted: the stored text
    // simply ends earlier, and the response reports exactly what it returned.
    let past_last = MAX_GIT_CONTENT_WINDOW_LINES as i32;
    assert!(
        entry
            .line_window(6, past_last, MAX_GIT_CONTENT_WINDOW_LINES)
            .is_ok()
    );
}

#[test]
fn a_window_page_returns_verbatim_text_and_exact_byte_count() {
    let rows = vec![
        chunk(1, 2, "fn one() {}\nfn two() {}\n"),
        chunk(3, 4, "\tfn three() {\t}\nfn four() {}\n"),
    ];
    let page = content_page(&rows, 2, 3, 0);
    assert_eq!(page.text, "fn two() {}\n\tfn three() {\t}\n");
    assert_eq!(
        page.byte_count,
        page.text.len() as i64,
        "the reported count is the exact UTF-8 length of the text"
    );
    assert_eq!(page.byte_count, 28);
    assert!(!page.pagination.has_more, "both chunks fit in one page");
    assert_eq!(page.pagination.next_cursor, None);

    // A window wholly inside one chunk trims only that chunk.
    let inside = content_page(&rows[..1], 2, 2, 0);
    assert_eq!(inside.text, "fn two() {}\n");
    // A window past the stored text returns what exists, and nothing more.
    let past_end = content_page(&rows, 90, 120, 0);
    assert_eq!(past_end.text, "");
    assert_eq!(past_end.byte_count, 0);
}

#[test]
fn a_page_stops_before_the_chunk_that_would_cross_the_byte_cap() {
    // Each stored chunk is far below the cap; together they cross it, so the
    // page ends on a chunk boundary and continuation is announced.
    let chunk_bytes = MAX_GIT_CONTENT_WINDOW_BYTES / 4;
    let text = "x".repeat(chunk_bytes);
    let rows: Vec<StoredGitGenerationChunk> = (1..=5)
        .map(|index| {
            let mut row = chunk(index, index, &text);
            row.chunk_index = index;
            row
        })
        .collect();
    // Four chunks fill the cap exactly, so the page stops before the fifth: a
    // page that ended mid-chunk could not be concatenated by the caller.
    let page = content_page(&rows, 1, 5, 0);
    assert_eq!(page.text.len(), chunk_bytes * 4);
    assert_eq!(page.byte_count, (chunk_bytes * 4) as i64);
    assert_eq!(
        page.text.len(),
        MAX_GIT_CONTENT_WINDOW_BYTES,
        "a page fills the cap but never crosses it"
    );
    assert!(page.pagination.has_more);
    assert_eq!(
        page.pagination.next_cursor.as_deref(),
        Some("4"),
        "the next page starts after the four chunks this one returned"
    );
    page.pagination
        .validate_continuation()
        .expect("has_more must carry its continuation");

    // The continuation reassembles the window in order, byte for byte. The
    // offset is applied by the statement, so a continuation page is handed the
    // rows that statement returned from that offset.
    let rest = content_page(&rows[4..], 1, 5, 4);
    assert_eq!(rest.text, text);
    assert!(!rest.pagination.has_more);
    assert_eq!(format!("{}{}", page.text, rest.text), text.repeat(5));
}

#[test]
fn a_page_ends_at_the_row_bound_and_keeps_the_extra_row_as_a_probe() {
    let rows: Vec<StoredGitGenerationChunk> = (0..=MAX_CHUNK_PAGE_ROWS)
        .map(|index| {
            let mut row = chunk(1, 1, "line\n");
            row.chunk_index = index as i32;
            row
        })
        .collect();
    let page = content_page(&rows, 1, 1, 0);
    assert_eq!(
        page.text.matches("line\n").count(),
        MAX_CHUNK_PAGE_ROWS as usize
    );
    assert!(
        page.pagination.has_more,
        "the unconsumed probe row means more matching chunks exist"
    );
    assert_eq!(
        page.pagination.next_cursor.as_deref(),
        Some(MAX_CHUNK_PAGE_ROWS.to_string().as_str())
    );
}

#[tokio::test]
async fn a_continuation_is_a_bounded_offset_this_service_issued() {
    assert_eq!(
        decode_cursor(None).expect("no cursor starts at the first row"),
        0
    );
    assert_eq!(decode_cursor(Some("0")).expect("first page"), 0);
    assert_eq!(
        decode_cursor(Some("4096")).expect("last chunk of a file"),
        4096
    );

    for malformed in [
        "",
        " 1",
        "1 ",
        "-1",
        "+1",
        "1.0",
        "abc",
        "0x10",
        "1_0",
        // A file holds at most the storage ceiling in chunks, so a larger
        // offset could never be one this service issued.
        "4097",
        "99999999999999999999999",
    ] {
        let refused = decode_cursor(Some(malformed))
            .expect_err("a cursor this service never issued must be refused");
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        let body = response_body(*refused).await;
        assert!(
            body.contains("git_content_cursor_invalid"),
            "cursor {malformed:?} must be refused as a bounded client error: {body}"
        );
    }
}
