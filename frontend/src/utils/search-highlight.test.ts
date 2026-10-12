import { describe, expect, it } from "vitest";

import {
  escapeSearchRegExp,
  findSearchHighlightRanges,
  highlightQueryText,
  highlightRenderedHtml,
  searchHighlightTerms,
} from "./search-highlight";

describe("searchHighlightTerms", () => {
  it("splits on whitespace and ASCII punctuation like the backend keyword_terms", () => {
    expect(searchHighlightTerms("deep seek")).toEqual(["deep", "seek"]);
    expect(searchHighlightTerms("deep-seek, model")).toEqual(["deep", "seek", "model"]);
    expect(searchHighlightTerms("https://example.com/a-b")).toEqual([
      "https",
      "example",
      "com",
      "a",
      "b",
    ]);
  });

  it("drops empty terms and case-insensitive duplicates", () => {
    expect(searchHighlightTerms("  Deep   deep  DEEP ")).toEqual(["Deep"]);
    expect(searchHighlightTerms("--- ***")).toEqual([]);
    expect(searchHighlightTerms("")).toEqual([]);
  });

  it("keeps CJK runs and non-ASCII letters intact", () => {
    expect(searchHighlightTerms("统一 大市场")).toEqual(["统一", "大市场"]);
  });
});

describe("findSearchHighlightRanges", () => {
  it("matches case-insensitively but reports offsets into the original text", () => {
    expect(findSearchHighlightRanges("DeepSeek release", ["deepseek"])).toEqual([
      { start: 0, end: 8 },
    ]);
  });

  it("keeps offsets stable when lowercasing would change the text length", () => {
    // `İ`.toLowerCase() expands to two code units, so a lowercased copy would
    // report the match one index too late.
    expect(findSearchHighlightRanges("İstanbul DeepSeek", ["deepseek"])).toEqual([
      { start: 9, end: 17 },
    ]);
  });

  it("keeps the longest match at a position instead of nesting marks", () => {
    expect(findSearchHighlightRanges("abcd", ["abcd", "bcd", "bc"])).toEqual([
      { start: 0, end: 4 },
    ]);
  });

  it("returns disjoint ranges for adjacent matches", () => {
    expect(findSearchHighlightRanges("DeepSeek", ["deep", "seek"])).toEqual([
      { start: 0, end: 4 },
      { start: 4, end: 8 },
    ]);
  });

  it("treats regex metacharacters as literals", () => {
    expect(findSearchHighlightRanges("a.*b and a*b", [".*"])).toEqual([{ start: 1, end: 3 }]);
    expect(findSearchHighlightRanges("a.b and axb", ["."])).toEqual([{ start: 1, end: 2 }]);
  });

  it("returns nothing for empty text or empty terms", () => {
    expect(findSearchHighlightRanges("", ["deep"])).toEqual([]);
    expect(findSearchHighlightRanges("deep", [])).toEqual([]);
  });
});

describe("escapeSearchRegExp", () => {
  it("escapes regex syntax characters only", () => {
    expect(escapeSearchRegExp("a.b*c$d(e)[f]{g}|h^i?j+k\\l")).toBe(
      "a\\.b\\*c\\$d\\(e\\)\\[f\\]\\{g\\}\\|h\\^i\\?j\\+k\\\\l",
    );
    expect(escapeSearchRegExp("plain-term")).toBe("plain-term");
  });
});

describe("highlightQueryText", () => {
  it("marks a case-insensitive term and preserves the original casing", () => {
    expect(highlightQueryText("DeepSeek release notes", "deepseek")).toBe(
      "<mark>DeepSeek</mark> release notes",
    );
  });

  it("marks every term of a multi-term query", () => {
    expect(highlightQueryText("DeepSeek model release", "deep model")).toBe(
      "<mark>Deep</mark>Seek <mark>model</mark> release",
    );
    // Terms that land next to each other produce adjacent marks, never nesting.
    expect(highlightQueryText("DeepSeek model", "deep seek")).toBe(
      "<mark>Deep</mark><mark>Seek</mark> model",
    );
  });

  it("splits query terms on ASCII punctuation the same way as the backend", () => {
    expect(highlightQueryText("DeepSeek-V3 weights", "deepseek-v3")).toBe(
      "<mark>DeepSeek</mark>-<mark>V3</mark> weights",
    );
  });

  it("escapes once and never lets the text or the query inject markup", () => {
    expect(highlightQueryText("<script>alert('DeepSeek')</script>", "deepseek")).toBe(
      "&lt;script&gt;alert(&#39;<mark>DeepSeek</mark>&#39;)&lt;/script&gt;",
    );
  });

  it("renders a markup-like query as text instead of matching a tag", () => {
    // "<b>" tokenizes to the single term "b", so the escaped text is marked
    // rather than the source markup being re-parsed as a tag.
    expect(highlightQueryText("DeepSeek <b>bold</b>", "<b>")).toBe(
      "DeepSeek &lt;<mark>b</mark>&gt;<mark>b</mark>old&lt;/<mark>b</mark>&gt;",
    );
  });

  it("returns escaped text unchanged for a blank or punctuation-only query", () => {
    expect(highlightQueryText("Deep & Seek", "")).toBe("Deep &amp; Seek");
    expect(highlightQueryText("Deep & Seek", "   ")).toBe("Deep &amp; Seek");
    expect(highlightQueryText("Deep & Seek", "---")).toBe("Deep &amp; Seek");
  });

  it("preserves newlines, CJK text and escaped entities", () => {
    expect(highlightQueryText("第一行\n第二行 & DeepSeek", "deepseek")).toBe(
      "第一行\n第二行 &amp; <mark>DeepSeek</mark>",
    );
  });

  it("marks a CJK substring", () => {
    expect(highlightQueryText("统一大市场建设方案", "大市场")).toBe(
      "统一<mark>大市场</mark>建设方案",
    );
  });
});

describe("highlightRenderedHtml", () => {
  it("marks text without touching tags or attribute values", () => {
    expect(
      highlightRenderedHtml(
        '<a href="https://example.com/deepseek" title="deepseek">DeepSeek link</a>',
        "deepseek",
      ),
    ).toBe(
      '<a href="https://example.com/deepseek" title="deepseek"><mark>DeepSeek</mark> link</a>',
    );
  });

  it("never marks inside an encoded entity", () => {
    expect(highlightRenderedHtml("<p>a &amp; DeepSeek</p>", "amp")).toBe(
      "<p>a &amp; DeepSeek</p>",
    );
    expect(highlightRenderedHtml("<p>&lt;b&gt;DeepSeek&lt;/b&gt;</p>", "lt")).toBe(
      "<p>&lt;b&gt;DeepSeek&lt;/b&gt;</p>",
    );
    // "amp" still cannot reach into `&amp;`, while the other term marks normally.
    expect(highlightRenderedHtml("<p>a &amp; DeepSeek</p>", "amp deepseek")).toBe(
      "<p>a &amp; <mark>DeepSeek</mark></p>",
    );
  });

  it("does not match a term that only exists once markup is stripped", () => {
    expect(highlightRenderedHtml("<p>Deep<b>Seek</b></p>", "deepseek")).toBe(
      "<p>Deep<b>Seek</b></p>",
    );
  });

  it("keeps code markup and its class attribute while marking code text", () => {
    expect(
      highlightRenderedHtml(
        '<pre class="library-markdown-pre"><code>const deepseek = 1;</code></pre>',
        "deepseek",
      ),
    ).toBe(
      '<pre class="library-markdown-pre"><code>const <mark>deepseek</mark> = 1;</code></pre>',
    );
  });

  it("marks heading text and leaves the heading tag untouched", () => {
    expect(highlightRenderedHtml("<h2>DeepSeek Report</h2>", "deepseek")).toBe(
      "<h2><mark>DeepSeek</mark> Report</h2>",
    );
  });

  it("leaves markup untouched for a blank or punctuation-only query", () => {
    expect(highlightRenderedHtml("<p>DeepSeek</p>", "")).toBe("<p>DeepSeek</p>");
    expect(highlightRenderedHtml("<p>DeepSeek</p>", "---")).toBe("<p>DeepSeek</p>");
  });

  it("copies an unterminated tag verbatim", () => {
    expect(highlightRenderedHtml("<p>DeepSeek<br", "deepseek")).toBe(
      "<p><mark>DeepSeek</mark><br",
    );
  });
});