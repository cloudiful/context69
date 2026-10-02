//! Request paging for the Git repository manifest listing (issue #681 phase
//! 5A).
//!
//! The manifest read is offset-paged, so this module owns both ends of the
//! continuation contract: it validates the shared cursor query, decodes the
//! offset this service issued, and re-encodes that offset as the next page's
//! token. Keeping the pair together is what makes the token unguessable-by-
//! accident rather than a caller-chosen position — a value that is not a
//! bounded decimal offset is a malformed request, not something to coerce.
//!
//! Nothing here knows what a manifest is; the caller decides what one page of
//! rows means, and asks only for the extra row that proves another page exists.

use crate::contracts::CursorPageQuery;
use crate::db::MAX_GIT_MANIFEST_FILES;
use crate::domain_errors::DomainError;

/// Longest accepted continuation cursor. The cursor is a decimal row offset, so
/// a bounded width keeps a hostile value from becoming a long parse or an
/// unbounded `OFFSET` in the database.
const MAX_CURSOR_CHARS: usize = 20;

/// The page one request asks for, already validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ManifestPage {
    pub(super) offset: i64,
    limit: i64,
}

impl ManifestPage {
    /// Entries this page returns at most.
    pub(super) fn limit(&self) -> i64 {
        self.limit
    }

    /// Rows to read: one more than the page holds, so an extra entry proves
    /// another page exists.
    pub(super) fn fetch_limit(&self) -> i64 {
        self.limit + 1
    }

    /// Offset the next page starts at, issued only when one exists.
    fn next_offset(&self) -> i64 {
        self.offset + self.limit
    }

    /// The continuation token for the page after this one, or `None` when this
    /// page reached the end of the set.
    pub(super) fn next_cursor(&self, has_more: bool) -> Option<String> {
        has_more.then(|| self.next_offset().to_string())
    }
}

/// Validate the shared cursor query shape and decode the continuation this
/// service issued. Both failures are bounded invalid-argument errors, decided
/// before any repository lookup.
pub(super) fn requested_page(query: &CursorPageQuery) -> anyhow::Result<ManifestPage> {
    context69_http_support::validate_cursor_limit(query.limit)?;
    let offset = match query.cursor.as_deref() {
        Some(cursor) => decode_offset(cursor)?,
        None => 0,
    };
    Ok(ManifestPage {
        offset,
        limit: i64::from(query.limit),
    })
}

/// Decode the server-issued offset cursor.
///
/// A continuation carries no signed or caller-chosen state, only the row offset
/// the previous page ended at, so it is accepted only as a bounded decimal
/// offset. Anything else is a malformed request rather than a value to coerce,
/// and the offset is capped at the largest manifest this storage layer writes.
fn decode_offset(cursor: &str) -> anyhow::Result<i64> {
    if cursor.is_empty()
        || cursor.len() > MAX_CURSOR_CHARS
        || !cursor.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(DomainError::invalid_argument("git_manifest_cursor_invalid").into());
    }
    let max_offset = MAX_GIT_MANIFEST_FILES as i64;
    match cursor
        .parse::<i64>()
        .ok()
        .filter(|offset| (0..=max_offset).contains(offset))
    {
        Some(offset) => Ok(offset),
        None => Err(DomainError::invalid_argument("git_manifest_cursor_out_of_bounds").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_CURSOR_CHARS, ManifestPage, decode_offset, requested_page};
    use crate::contracts::CursorPageQuery;
    use crate::db::MAX_GIT_MANIFEST_FILES;

    fn query(limit: u32, cursor: Option<&str>) -> CursorPageQuery {
        CursorPageQuery {
            limit,
            cursor: cursor.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn a_bounded_limit_starts_at_the_first_offset() {
        let page = requested_page(&query(1, None)).expect("limit 1 is in range");
        assert_eq!(page.offset, 0);
        assert_eq!(page.fetch_limit(), 2, "one extra row proves has_more");
        assert_eq!(page.next_cursor(true).as_deref(), Some("1"));
        assert_eq!(page.next_cursor(false), None);

        let page = requested_page(&query(100, None)).expect("limit 100 is in range");
        assert_eq!(page.offset, 0);
        assert_eq!(page.fetch_limit(), 101);
        assert_eq!(page.next_cursor(true).as_deref(), Some("100"));
    }

    #[test]
    fn out_of_range_limits_are_bounded_invalid_arguments() {
        for limit in [0, 101, u32::MAX] {
            let error = requested_page(&query(limit, None))
                .expect_err("an out-of-range limit must be rejected")
                .to_string();
            assert!(
                error.contains("limit must be between 1 and 100"),
                "limit {limit} must report the shared bound: {error}"
            );
        }
    }

    #[test]
    fn only_a_server_issued_offset_cursor_decodes() {
        assert_eq!(decode_offset("0").expect("first page"), 0);
        assert_eq!(decode_offset("250").expect("continued page"), 250);
        assert_eq!(
            decode_offset(&(MAX_GIT_MANIFEST_FILES as i64).to_string())
                .expect("a manifest cannot be offset past its own bound"),
            MAX_GIT_MANIFEST_FILES as i64
        );

        for malformed in [
            "", " 1", "1 ", "-1", "+1", "1.0", "abc", "0x10", "1_0", "١٢",
        ] {
            let error = decode_offset(malformed)
                .expect_err("a cursor the handler never issued must be rejected")
                .to_string();
            assert!(
                error.contains("git_manifest_cursor_invalid"),
                "cursor {malformed:?} must be rejected as malformed: {error}"
            );
        }

        let oversized = "9".repeat(MAX_CURSOR_CHARS + 1);
        assert!(
            decode_offset(&oversized)
                .expect_err("an oversized cursor must be rejected")
                .to_string()
                .contains("git_manifest_cursor_invalid")
        );
        let past_manifest = (MAX_GIT_MANIFEST_FILES as i64 + 1).to_string();
        assert!(
            decode_offset(&past_manifest)
                .expect_err("an offset past the manifest bound must be rejected")
                .to_string()
                .contains("git_manifest_cursor_out_of_bounds")
        );
    }

    #[test]
    fn a_malformed_cursor_is_rejected_before_any_lookup() {
        let error = requested_page(&query(50, Some("not-a-cursor")))
            .expect_err("a malformed continuation must not page")
            .to_string();
        assert!(
            error.contains("git_manifest_cursor_invalid"),
            "a malformed cursor is a bounded invalid argument: {error}"
        );
    }

    #[test]
    fn a_continued_page_reports_only_its_own_window() {
        let page = requested_page(&query(2, Some("4"))).expect("continuation");
        assert_eq!(page.offset, 4);
        assert_eq!(page.fetch_limit(), 3);
        assert_eq!(page.next_cursor(true).as_deref(), Some("6"));
        // The token is a window position, not a request replay: the same cursor
        // under a different limit continues from the offset it carries.
        assert_eq!(
            ManifestPage {
                offset: 4,
                limit: 2
            }
            .next_offset(),
            6
        );
    }
}
