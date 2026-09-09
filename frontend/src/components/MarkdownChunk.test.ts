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
});
