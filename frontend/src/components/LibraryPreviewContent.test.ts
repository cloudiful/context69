import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
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
    expect(wrapper.find("pre").exists()).toBe(false);
  });

  it("renders plain text content in a pre block", () => {
    const wrapper = mount(LibraryPreviewContent, {
      props: {
        content: "# Title\n\n**Bold** body",
        contentFormat: "plain_text",
      },
    });

    expect(wrapper.find("pre").exists()).toBe(true);
    expect(wrapper.text()).toContain("# Title");
    expect(wrapper.find(".library-markdown-content").exists()).toBe(false);
  });

  it("paginates markdown by semantic blocks without adding a vertical content scroll", async () => {
    const content = Array.from({ length: 20 }, (_, index) => `# Section ${index + 1}`).join("\n\n");
    const wrapper = mount(LibraryPreviewContent, {
      props: { content, contentFormat: "markdown" },
      global: { plugins: [testNuxtUiPlugin] },
    });

    await vi.waitFor(() => {
      expect(wrapper.findComponent({ name: "Pagination" }).exists()).toBe(true);
    });

    expect(wrapper.find(".library-markdown-content").classes()).not.toContain("overflow-auto");
    expect(wrapper.text()).toContain("Section 1");

    const pagination = wrapper.findComponent({ name: "Pagination" });
    pagination.vm.$emit("update:page", 2);
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).toContain("Section 10");
    expect(wrapper.text()).not.toContain("Section 2");
  });
});
