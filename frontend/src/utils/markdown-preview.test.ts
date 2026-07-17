import { describe, expect, it } from "vitest";

import { renderMarkdownPreview, renderMarkdownPreviewBlocks } from "./markdown-preview";
import { paginateMarkdownPreview } from "./markdown-preview-pagination";

describe("renderMarkdownPreview", () => {
  it("renders headings, lists and links", () => {
    const html = renderMarkdownPreview([
      "# Title",
      "",
      "Paragraph with [link](https://example.com).",
      "",
      "- first",
      "- second",
    ].join("\n"));

    expect(html).toContain("<h1>Title</h1>");
    expect(html).toContain("<p>Paragraph with <a href=\"https://example.com\"");
    expect(html).toContain("<ul><li>first</li><li>second</li></ul>");
  });

  it("escapes unsafe html instead of injecting it", () => {
    const html = renderMarkdownPreview("<script>alert('xss')</script>\n\nSafe");

    expect(html).toContain("&lt;script&gt;alert");
    expect(html).not.toContain("<script>");
  });

  it("keeps semantic blocks intact when paginating", () => {
    const blocks = renderMarkdownPreviewBlocks("# Title\n\nParagraph");
    const pages = paginateMarkdownPreview(blocks, 2);

    expect(pages).toHaveLength(2);
    expect(pages[0]).toHaveLength(1);
    expect(pages[0][0].html).toContain("<h1>Title</h1>");
    expect(pages[1][0].html).toContain("<p>Paragraph</p>");
  });
});
