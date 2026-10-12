/**
 * Shared query-term tokenizer and safe highlighter.
 *
 * Result titles, result snippets and markdown previews all mark the same query
 * terms with the same rules, so a term behaves identically in every view. The
 * tokenizer mirrors the backend `keyword_terms` splitter in `src/db/rows.rs`:
 * split on whitespace or ASCII punctuation, drop empty terms, and dedupe
 * case-insensitively.
 */

export function escapeSearchHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function escapeSearchRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const WHITESPACE_PATTERN = /\s/;

/**
 * Whitespace or ASCII punctuation. Mirrors Rust's `char::is_ascii_punctuation`
 * over `!`..`/`, `:`..`@`, `[`..`` ` `` and `{`..`~`, so digits and ASCII letters
 * stay part of a term.
 */
function isTermSeparator(ch: string): boolean {
  if (WHITESPACE_PATTERN.test(ch)) {
    return true;
  }
  const code = ch.codePointAt(0) ?? 0;
  if (code < 0x21 || code > 0x7e) {
    return false;
  }
  if (code >= 0x30 && code <= 0x39) {
    return false;
  }
  return !(code >= 0x41 && code <= 0x5a) && !(code >= 0x61 && code <= 0x7a);
}

export function searchHighlightTerms(query: string): string[] {
  const terms: string[] = [];
  const seen = new Set<string>();
  let current = "";

  const flush = (): void => {
    if (!current) {
      return;
    }
    const key = current.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      terms.push(current);
    }
    current = "";
  };

  for (const ch of query) {
    if (isTermSeparator(ch)) {
      flush();
      continue;
    }
    current += ch;
  }
  flush();

  return terms;
}

export interface SearchHighlightRange {
  start: number;
  end: number;
}

/**
 * Case-insensitive literal occurrences of `terms` in `text`, as offsets into the
 * original string. Matching runs through escaped-regex literals so offsets never
 * depend on a case-folded copy of the text: a naive `toLowerCase()` changes the
 * length of characters such as `İ` and would shift every later index.
 *
 * Candidates are accepted left to right, longest first at each position, so the
 * returned ranges never overlap and a mark is never nested inside another one.
 */
export function findSearchHighlightRanges(
  text: string,
  terms: string[],
): SearchHighlightRange[] {
  const patterns = new Set(
    terms.filter((term) => term.length > 0).map((term) => escapeSearchRegExp(term)),
  );
  if (!text || patterns.size === 0) {
    return [];
  }

  const candidates: SearchHighlightRange[] = [];
  for (const pattern of patterns) {
    const regexp = new RegExp(pattern, "giu");
    for (let match = regexp.exec(text); match; match = regexp.exec(text)) {
      candidates.push({ start: match.index, end: match.index + match[0].length });
    }
  }
  candidates.sort(
    (left, right) =>
      left.start - right.start || (right.end - right.start) - (left.end - left.start),
  );

  const accepted: SearchHighlightRange[] = [];
  let cursor = 0;
  for (const range of candidates) {
    if (range.start < cursor) {
      continue;
    }
    accepted.push(range);
    cursor = range.end;
  }
  return accepted;
}

function keepAsIs(value: string): string {
  return value;
}

/**
 * Wraps `ranges` of `text` in `<mark>`, passing every emitted slice through
 * `escape` exactly once. Escaping per slice rather than escaping the whole
 * string first guarantees that a mark can never split an escaped entity and
 * that nothing is escaped twice.
 */
function renderMarkedText(
  text: string,
  ranges: SearchHighlightRange[],
  escape: (value: string) => string,
): string {
  if (ranges.length === 0) {
    return escape(text);
  }

  let output = "";
  let cursor = 0;
  for (const range of ranges) {
    output += escape(text.slice(cursor, range.start));
    output += `<mark>${escape(text.slice(range.start, range.end))}</mark>`;
    cursor = range.end;
  }
  return output + escape(text.slice(cursor));
}

/**
 * Highlights query terms in plain text and escapes the result for `v-html`.
 * The displayed casing, whitespace and CJK characters of the source text are
 * preserved; only the marked spans are wrapped.
 */
export function highlightQueryText(text: string, query: string): string {
  const terms = searchHighlightTerms(query);
  return renderMarkedText(text, findSearchHighlightRanges(text, terms), escapeSearchHtml);
}

const HTML_ENTITY_PATTERN = /^&(?:#\d+|#x[\da-fA-F]+|[a-zA-Z][a-zA-Z0-9]*);/;

/**
 * Marks a run of already-escaped HTML text. Encoded entities are copied
 * verbatim and excluded from matching, so a query can never match — or split —
 * an entity such as `&amp;`. Matching inside those entities would both render
 * wrongly and let a query reach into the escape layer.
 */
function markEscapedRun(run: string, terms: string[]): string {
  let output = "";
  let plainStart = 0;
  let index = 0;

  while (index < run.length) {
    if (run[index] !== "&") {
      index += 1;
      continue;
    }
    const entity = HTML_ENTITY_PATTERN.exec(run.slice(index));
    if (!entity) {
      index += 1;
      continue;
    }
    const plain = run.slice(plainStart, index);
    output += renderMarkedText(plain, findSearchHighlightRanges(plain, terms), keepAsIs);
    output += entity[0];
    index += entity[0].length;
    plainStart = index;
  }

  const tail = run.slice(plainStart);
  return output + renderMarkedText(tail, findSearchHighlightRanges(tail, terms), keepAsIs);
}

/**
 * Highlights query terms in already-rendered, already-escaped HTML such as the
 * output of `renderMarkdownPreviewBlocks`. Tags are copied verbatim, so markup,
 * tag names and attribute values are never marked, and nothing is escaped a
 * second time.
 */
export function highlightRenderedHtml(html: string, query: string): string {
  const terms = searchHighlightTerms(query);
  if (!html || terms.length === 0) {
    return html;
  }

  let output = "";
  let index = 0;
  while (index < html.length) {
    const tagStart = html.indexOf("<", index);
    if (tagStart === -1) {
      output += markEscapedRun(html.slice(index), terms);
      break;
    }
    output += markEscapedRun(html.slice(index, tagStart), terms);
    const tagEnd = html.indexOf(">", tagStart);
    if (tagEnd === -1) {
      output += html.slice(tagStart);
      break;
    }
    output += html.slice(tagStart, tagEnd + 1);
    index = tagEnd + 1;
  }
  return output;
}