import { describe, expect, it } from "vitest";

import { renderMarkdownPreview } from "./markdown-preview";

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
});
