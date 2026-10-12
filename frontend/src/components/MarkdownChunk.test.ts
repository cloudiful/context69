import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

import MarkdownChunk from "./MarkdownChunk.vue";

describe("MarkdownChunk", () => {
  it("renders markdown content with formatted html", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "# Title\n\n**Bold** body",
        markdown: true,
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    expect(wrapper.html()).toContain("<h1>Title</h1>");
    expect(wrapper.html()).toContain("<strong>Bold</strong>");
    expect(wrapper.find("pre").exists()).toBe(false);
  });

  it("renders plain text in a pre block", () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "# Title\n\n**Bold** body",
        markdown: false,
      },
    });

    expect(wrapper.find("pre").exists()).toBe(true);
    expect(wrapper.text()).toContain("# Title");
    expect(wrapper.find(".library-markdown-content").exists()).toBe(false);
  });

  it("highlights query terms in markdown text without touching tags", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "## Report\n\nThe policy update applies broadly.",
        markdown: true,
        highlight: "policy",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    const html = wrapper.find(".library-markdown-content").html();
    expect(html).toContain("<mark>policy</mark>");
    expect(html).toContain("<h2>Report</h2>");
  });

  it("highlights query terms in the plain pre fallback", () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "the policy text",
        markdown: false,
        highlight: "policy",
      },
    });

    const pre = wrapper.find("pre");
    expect(pre.exists()).toBe(true);
    expect(pre.html()).toContain("<mark>policy</mark>");
    expect(pre.text()).toContain("the policy text");
  });

  it("marks every query term case-insensitively in rendered markdown", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "## Report\n\nThe Policy update applies broadly, policy included.",
        markdown: true,
        highlight: "policy applies",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    const html = wrapper.find(".library-markdown-content").html();
    expect(html).toContain("<mark>Policy</mark>");
    expect(html).toContain("<mark>applies</mark>");
    expect(html).toContain("<mark>policy</mark>");
    expect(html).toContain("<h2>Report</h2>");
  });

  it("marks fenced code text while leaving the code block markup intact", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "Run the deepseek CLI first.\n\n```\ndeepseek --flag\n```",
        markdown: true,
        highlight: "deepseek",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    const html = wrapper.find(".library-markdown-content").html();
    expect(html).toContain('<pre class="library-markdown-pre"><code>');
    expect(html).toContain("<mark>deepseek</mark> CLI");
    expect(html).toContain("<code><mark>deepseek</mark> --flag</code>");
  });

  it("leaves link and code attributes unmarked in pre-rendered html", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "ignored",
        markdown: true,
        highlight: "deepseek",
        html: '<p><a href="https://example.com/deepseek" title="deepseek">deepseek docs</a></p>',
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    const html = wrapper.find(".library-markdown-content").html();
    expect(html).toContain('href="https://example.com/deepseek"');
    expect(html).toContain('title="deepseek"');
    expect(html).toContain("><mark>deepseek</mark> docs</a>");
  });

  it("never marks inside an encoded entity in rendered markdown", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "Use a & b in <tags>.",
        markdown: true,
        highlight: "lt amp",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    const html = wrapper.find(".library-markdown-content").html();
    expect(html).toContain("a &amp; b in &lt;tags&gt;.");
    expect(html).not.toContain("<mark>amp</mark>");
    expect(html).not.toContain("<mark>lt</mark>");
  });

  it("renders dangerous markdown input as text and still marks the match", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: '<img src=x onerror="alert(DeepSeek)">',
        markdown: true,
        highlight: "deepseek",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    expect(wrapper.find("img").exists()).toBe(false);
    const article = wrapper.find(".library-markdown-content");
    // The renderer escapes once; jsdom re-serializes text content with a bare
    // quote, so the outcome is asserted at the DOM level rather than as a raw
    // entity string. No double escaping and no markup injection.
    expect(article.element.innerHTML).not.toContain("&amp;lt;");
    expect(article.text()).toBe('<img src=x onerror="alert(DeepSeek)">');
    expect(article.html()).toContain("<mark>DeepSeek</mark>");
  });

  it("uses the same shared matcher for the plain pre fallback", () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "The DeepSeek model release",
        markdown: false,
        highlight: "deepseek release",
      },
    });

    const pre = wrapper.find("pre");
    expect(pre.html()).toContain("<mark>DeepSeek</mark>");
    expect(pre.html()).toContain("<mark>release</mark>");
    expect(pre.text()).toBe("The DeepSeek model release");
  });

  it("renders the plain pre fallback unescaped twice", () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "a & b < c",
        markdown: false,
        highlight: "deepseek",
      },
    });

    const pre = wrapper.find("pre");
    expect(pre.html()).toContain("a &amp; b &lt; c");
    expect(pre.html()).not.toContain("&amp;amp;");
  });

  it("drops stale marks when the highlight prop changes", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "## Report\n\nThe policy update applies.",
        markdown: true,
        highlight: "policy",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });
    expect(wrapper.find(".library-markdown-content").html()).toContain("<mark>policy</mark>");

    await wrapper.setProps({ highlight: "report" });
    const updated = wrapper.find(".library-markdown-content").html();
    expect(updated).not.toContain("<mark>policy</mark>");
    expect(updated).toContain("<mark>Report</mark>");
  });

  it("marks real markdown link and inline code text without corrupting content", async () => {
    const wrapper = mount(MarkdownChunk, {
      props: {
        content: "DeepSeek update: see [DeepSeek report](https://example.com/deepseek) and `deepseek --flag` for details.",
        markdown: true,
        highlight: "deepseek report",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    const html = wrapper.find(".library-markdown-content").html();
    expect(html).not.toContain("@@LINK");
    expect(html).not.toContain("@@CODE");
    expect(html).not.toContain("<em>");
    expect(html).toContain('href="https://example.com/deepseek"');
    expect(html).toContain("<code>");
    expect(html).toContain("<mark>DeepSeek</mark>");
    expect(html).toContain("<mark>deepseek</mark>");
    expect(html).not.toContain('href="https://example.com/<mark>');
  });
});
