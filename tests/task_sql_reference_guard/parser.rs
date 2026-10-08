//! The read-only lexer that finds the path argument of a task SQL macro.
//!
//! It recognizes exactly `sqlx::query_file!`, `sqlx::query_file_as!`,
//! `sqlx::query_file_scalar!`, and `include_str!`, skips comments and strings,
//! and records only the dedicated path argument — never a later bind value or a
//! nested macro argument.

/// A recognized macro invocation, carrying both which argument holds the SQL
/// path and how that path is resolved.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlMacro {
    /// `sqlx::query_file!` / `sqlx::query_file_scalar!`: the first argument is
    /// the path, resolved relative to the crate root.
    QueryFileFirst,
    /// `sqlx::query_file_as!`: a row type precedes the path, so the second
    /// argument is the path, resolved relative to the crate root.
    QueryFileAsSecond,
    /// `include_str!`: the first argument is the path, resolved relative to the
    /// Rust source file that contains the macro.
    IncludeStr,
}

/// Recognizes exactly the macro paths this repository embeds task SQL through.
/// A different namespace (`fake::query_file!`) or an unlisted name is not one.
fn recognized_macro(segments: &[String]) -> Option<SqlMacro> {
    match segments {
        [only] if only == "include_str" => Some(SqlMacro::IncludeStr),
        [namespace, name] if namespace == "sqlx" => match name.as_str() {
            "query_file" | "query_file_scalar" => Some(SqlMacro::QueryFileFirst),
            "query_file_as" => Some(SqlMacro::QueryFileAsSecond),
            _ => None,
        },
        _ => None,
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_cont(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Skips whitespace and line/block (nested) comments.
fn skip_trivia(bytes: &[u8], mut i: usize) -> usize {
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            let mut nesting = 1usize;
            while i < bytes.len() && nesting > 0 {
                if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'*') {
                    nesting += 1;
                    i += 2;
                } else if bytes.get(i) == Some(&b'*') && bytes.get(i + 1) == Some(&b'/') {
                    nesting -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        return i;
    }
}

/// Reads one string literal starting at `i`: `"..."`, `r"..."`, or `r#"..."#`.
/// A `b`-prefixed byte form is read from its quote by the caller. Returns the
/// contents and the index just past the closing delimiter.
fn read_string(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    if bytes.get(i) == Some(&b'"') {
        let mut body = String::new();
        let mut j = i + 1;
        while j < bytes.len() {
            match bytes[j] {
                b'\\' => j += 2,
                b'"' => return Some((body, j + 1)),
                byte => {
                    body.push(byte as char);
                    j += 1;
                }
            }
        }
        return None;
    }
    if bytes.get(i) == Some(&b'r') {
        let mut j = i + 1;
        let mut hashes = 0usize;
        while bytes.get(j) == Some(&b'#') {
            hashes += 1;
            j += 1;
        }
        if bytes.get(j) != Some(&b'"') {
            return None;
        }
        j += 1;
        let content_start = j;
        let mut terminator = vec![b'"'];
        terminator.extend(std::iter::repeat_n(b'#', hashes));
        while j + terminator.len() <= bytes.len() {
            if bytes[j..j + terminator.len()] == terminator[..] {
                let body = String::from_utf8_lossy(&bytes[content_start..j]).into_owned();
                return Some((body, j + terminator.len()));
            }
            j += 1;
        }
    }
    None
}

/// Reads a `::`-separated path starting at `start`, returning its segments and
/// the index after the last segment (trivia skipped).
fn read_path(source: &str, bytes: &[u8], start: usize) -> (Vec<String>, usize) {
    let mut segments = Vec::new();
    let mut i = start;
    loop {
        let segment_start = i;
        while i < bytes.len() && is_ident_cont(bytes[i]) {
            i += 1;
        }
        segments.push(source[segment_start..i].to_string());
        let after = skip_trivia(bytes, i);
        if bytes.get(after) == Some(&b':') && bytes.get(after + 1) == Some(&b':') {
            let next = skip_trivia(bytes, after + 2);
            if bytes.get(next).is_some_and(|byte| is_ident_start(*byte)) {
                i = next;
                continue;
            }
        }
        return (segments, after);
    }
}

/// Skips the first argument of a macro invocation up to and including the
/// top-level comma, so the following argument can be read. Balanced `()[]{}<>`
/// are respected, so a comma inside a tuple or generic type is not mistaken for
/// the separator.
fn skip_first_argument(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut round = 0i32;
    let mut square = 0i32;
    let mut curly = 0i32;
    let mut angle = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'r' => match read_string(bytes, i) {
                Some((_, next)) => i = next,
                None => i += 1,
            },
            b'\'' => {
                // A char literal (`'x'`, `'\n'`) or a lifetime (`'a`).
                let escaped = bytes.get(i + 1) == Some(&b'\\');
                let single = bytes.get(i + 2) == Some(&b'\'');
                i += if escaped || single { 3 } else { 1 };
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_trivia(bytes, i),
            b'(' => {
                round += 1;
                i += 1;
            }
            b')' if round == 0 => return None,
            b')' => {
                round -= 1;
                i += 1;
            }
            b'[' => {
                square += 1;
                i += 1;
            }
            b']' => {
                square = square.saturating_sub(1);
                i += 1;
            }
            b'{' => {
                curly += 1;
                i += 1;
            }
            b'}' => {
                curly = curly.saturating_sub(1);
                i += 1;
            }
            b'<' => {
                angle += 1;
                i += 1;
            }
            b'>' => {
                angle = angle.saturating_sub(1);
                i += 1;
            }
            b',' if round == 0 && square == 0 && curly == 0 && angle == 0 => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Reads the path argument of a recognized macro: the first argument, or the
/// second for `query_file_as!` (whose first argument is the row type). Only a
/// bare string literal counts, so a nested `format!(...)` argument is not a
/// call-site.
fn argument_path(bytes: &[u8], i: usize, kind: SqlMacro) -> Option<String> {
    let mut i = skip_trivia(bytes, i);
    if kind == SqlMacro::QueryFileAsSecond {
        i = skip_first_argument(bytes, i)?;
        i = skip_trivia(bytes, i);
    }
    read_string(bytes, i).map(|(literal, _)| literal)
}

/// Every path literal of a recognized `sqlx::query_file*!`/`include_str!`
/// invocation, tagged with the macro so its resolution base is known.
///
/// This is a small read-only lexer, not a substring search: comments and
/// strings are skipped, only the exact macro paths are recognized, and only the
/// dedicated path argument is recorded. Later bind arguments and nested macro
/// arguments are never collected.
pub(crate) fn macro_path_literals(source: &str) -> Vec<(SqlMacro, String)> {
    let bytes = source.as_bytes();
    let mut literals = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        i = skip_trivia(bytes, i);
        if i >= bytes.len() {
            break;
        }
        let string_start = match bytes[i] {
            b'"' | b'r' => Some(i),
            b'b' if matches!(bytes.get(i + 1), Some(&b'"') | Some(&b'r')) => Some(i + 1),
            _ => None,
        };
        if let Some(start) = string_start
            && let Some((_, next)) = read_string(bytes, start)
        {
            i = next;
            continue;
        }
        if is_ident_start(bytes[i]) {
            let (segments, after_path) = read_path(source, bytes, i);
            let after_trivia = skip_trivia(bytes, after_path);
            if bytes.get(after_trivia) == Some(&b'!') {
                let open = skip_trivia(bytes, after_trivia + 1);
                if bytes.get(open) == Some(&b'(')
                    && let Some(kind) = recognized_macro(&segments)
                {
                    if let Some(literal) = argument_path(bytes, open + 1, kind) {
                        literals.push((kind, literal));
                    }
                    i = open + 1;
                    continue;
                }
            }
            i = after_path;
            continue;
        }
        i += 1;
    }
    literals
}
