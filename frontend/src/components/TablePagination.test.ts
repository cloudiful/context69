import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import TablePagination from "./TablePagination.vue";

describe("TablePagination", () => {
  it("pages from backend totals for legacy responses", async () => {
    const wrapper = mount(TablePagination, {
      props: {
        pagination: { page: 1, page_size: 8, total: 20, total_pages: 3 },
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const pagination = wrapper.findComponent({ name: "Pagination" });
    expect(pagination.exists()).toBe(true);
    expect(pagination.props("total")).toBe(20);
    expect(pagination.props("page")).toBe(1);

    await pagination.vm.$emit("update:page", 2);
    expect(wrapper.emitted("update:page")?.[0]).toEqual([2]);
  });

  it("keeps working when search window signals are present", async () => {
    const wrapper = mount(TablePagination, {
      props: {
        pagination: {
          page: 1,
          page_size: 8,
          total: 9,
          total_pages: 2,
          has_more: true,
          total_is_exact: false,
        },
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const pagination = wrapper.findComponent({ name: "Pagination" });
    expect(pagination.exists()).toBe(true);
    expect(pagination.props("total")).toBe(9);

    await pagination.vm.$emit("update:page", 2);
    expect(wrapper.emitted("update:page")?.[0]).toEqual([2]);
  });

  it("does not fabricate pagination when the window fits on one page", () => {
    const wrapper = mount(TablePagination, {
      props: {
        pagination: {
          page: 1,
          page_size: 8,
          total: 5,
          total_pages: 1,
          has_more: false,
          total_is_exact: false,
        },
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    expect(wrapper.findComponent({ name: "Pagination" }).exists()).toBe(false);
  });

  it("keeps working when has_more is absent for capped windows", async () => {
    const wrapper = mount(TablePagination, {
      props: {
        pagination: {
          page: 1,
          page_size: 8,
          total: 2000,
          total_pages: 250,
          total_is_exact: false,
        },
      },
      global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
    });

    const pagination = wrapper.findComponent({ name: "Pagination" });
    expect(pagination.exists()).toBe(true);
    expect(pagination.props("total")).toBe(2000);

    await pagination.vm.$emit("update:page", 2);
    expect(wrapper.emitted("update:page")?.[0]).toEqual([2]);
  });
});
