//! Tests for the `get_code` adapter (issue #681 phase 5I).
//!
//! Window assembly over synthetic stored chunks and the argument-validation
//! refusal surface, with no database and no provider.

use chrono::Utc;
use uuid::Uuid;

use super::{checked_get_args, window_page};
use crate::contracts::McpGetCodeRequest;
use crate::contracts::{MCP_CODE_GET_CHUNK_LIMIT_DEFAULT, MCP_CODE_GET_CHUNK_LIMIT_MAX};
use crate::db::StoredGitGenerationChunk;

fn chunk(index: i32, start_line: i32, end_line: i32, text: &str) -> StoredGitGenerationChunk {
    StoredGitGenerationChunk {
        chunk_key: Uuid::new_v4(),
        generation_key: Uuid::new_v4(),
        file_key: Uuid::new_v4(),
        chunk_index: index,
        start_line,
        end_line,
        text: text.to_string(),
        created_at: Utc::now(),
    }
}

fn base_request() -> McpGetCodeRequest {
    McpGetCodeRequest {
        group_path: "research/context69".to_string(),
        repository_key: Uuid::new_v4(),
        path: "src/mcp/tools/code.rs".to_string(),
        start_line: 1,
        end_line: 20,
        chunk_limit: MCP_CODE_GET_CHUNK_LIMIT_DEFAULT,
    }
}

#[test]
fn window_page_concatenates_the_requested_lines_in_chunk_order() {
    let rows = vec![
        chunk(0, 1, 2, "line1\nline2\n"),
        chunk(1, 3, 4, "line3\nline4\n"),
    ];
    let page = window_page(&rows, 1, 3, 64);
    assert_eq!(page.text, "line1\nline2\nline3\n");
    assert_eq!(page.byte_count, page.text.len() as i64);
    assert!(!page.truncated, "the window fits the chunk and byte caps");
}

#[test]
fn window_page_preserves_crlf_trailing_whitespace_and_utf8() {
    let rows = vec![chunk(0, 1, 2, "café\r\nx \n")];
    let page = window_page(&rows, 1, 2, 64);
    assert_eq!(page.text, "café\r\nx \n", "bytes stay verbatim");
    assert_eq!(page.byte_count, page.text.len() as i64);
    assert!(!page.truncated);
}

#[test]
fn window_page_truncates_at_the_chunk_limit() {
    let rows = vec![
        chunk(0, 1, 1, "a"),
        chunk(1, 1, 1, "b"),
        chunk(2, 1, 1, "c"),
    ];
    let page = window_page(&rows, 1, 1, 2);
    assert_eq!(page.text, "ab");
    assert!(page.truncated, "the extra probe row reports the drop");
}

#[test]
fn window_page_truncates_at_the_byte_cap() {
    let piece = "x".repeat(16 * 1024);
    let rows: Vec<StoredGitGenerationChunk> =
        (0..5).map(|index| chunk(index, 1, 1, &piece)).collect();
    let page = window_page(&rows, 1, 1, 64);
    assert_eq!(page.byte_count, 64 * 1024, "four chunks fill the byte cap");
    assert_eq!(page.text.len(), 64 * 1024);
    assert!(page.truncated, "the fifth chunk crossed the byte cap");
}

#[test]
fn checked_get_args_accepts_a_bounded_safe_window() {
    assert!(checked_get_args(&base_request()).is_ok());

    let mut widest = base_request();
    widest.end_line = 400;
    assert!(checked_get_args(&widest).is_ok());
}

#[test]
fn checked_get_args_rejects_unsafe_paths_and_bad_windows() {
    let reject = |label: &str, mutate: fn(&mut McpGetCodeRequest)| {
        let mut request = base_request();
        mutate(&mut request);
        assert!(checked_get_args(&request).is_err(), "must reject {label}");
    };

    reject("blank group_path", |request| {
        request.group_path = "  ".to_string();
    });
    reject("blank path", |request| request.path = String::new());
    reject("absolute path", |request| {
        request.path = "/etc/passwd".to_string()
    });
    reject("traversing path", |request| {
        request.path = "src/../../etc".to_string()
    });
    reject("dot segment", |request| request.path = "./src".to_string());
    reject("double slash", |request| {
        request.path = "src//lib".to_string()
    });
    reject("backslash segment", |request| {
        request.path = "src\\lib".to_string()
    });
    reject("control character", |request| {
        request.path = "src/\u{1}lib".to_string()
    });
    reject("zero start_line", |request| request.start_line = 0);
    reject("zero end_line", |request| request.end_line = 0);
    reject("reversed window", |request| {
        request.start_line = 10;
        request.end_line = 5;
    });
    reject("window wider than 400 lines", |request| {
        request.start_line = 1;
        request.end_line = 401;
    });
    reject("chunk_limit below minimum", |request| {
        request.chunk_limit = 0
    });
    reject("chunk_limit above maximum", |request| {
        request.chunk_limit = MCP_CODE_GET_CHUNK_LIMIT_MAX + 1;
    });
}
