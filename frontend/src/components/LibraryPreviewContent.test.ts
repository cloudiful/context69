import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

import LibraryPreviewContent from "./LibraryPreviewContent.vue";

describe("LibraryPreviewContent", () => {
  it("renders markdown content with formatted html", async () => {
    const wrapper = mount(LibraryPreviewContent, {
      props: {
        content: "# Title\n\n**Bold** body",
        contentFormat: "markdown",
      },
    });

    await vi.waitFor(() => {
      expect(wrapper.find(".library-markdown-content").exists()).toBe(true);
    });

    expect(wrapper.html()).toContain("<h1>Title</h1>");
    expect(wrapper.html()).toContain("<strong>Bold</strong>");
    expect(wrapper.find("pre.library-preview-plaintext").exists()).toBe(false);
  });

  it("renders plain text content in a pre block", () => {
    const wrapper = mount(LibraryPreviewContent, {
      props: {
        content: "# Title\n\n**Bold** body",
        contentFormat: "plain_text",
      },
    });

    expect(wrapper.find("pre.library-preview-plaintext").exists()).toBe(true);
    expect(wrapper.text()).toContain("# Title");
    expect(wrapper.find(".library-markdown-content").exists()).toBe(false);
  });
});
