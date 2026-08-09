import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { describe, expect, it, vi } from "vitest";

import type { PaginatedResponse } from "../services/api";
import { useServerPagination, type ServerPageLoader } from "./use-server-pagination";

function response(page: number, pageSize: number, items: number[], total = items.length): PaginatedResponse<number> {
  return {
    items,
    pagination: {
      page,
      page_size: pageSize,
      total,
      total_pages: total === 0 ? 0 : Math.ceil(total / pageSize),
    },
  };
}

describe("useServerPagination", () => {
  it("exposes loading while a request is pending", async () => {
    let resolveRequest!: (value: PaginatedResponse<number>) => void;
    const loader = vi.fn<ServerPageLoader<number>>(() => new Promise((resolve) => {
      resolveRequest = resolve;
    }));
    let state!: ReturnType<typeof useServerPagination<number>>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useServerPagination(loader);
        return {};
      },
      template: "<div />",
    }));

    const pending = state.load();
    expect(state.loading.value).toBe(true);
    resolveRequest(response(1, 50, [1]));
    await pending;
    expect(state.loading.value).toBe(false);
    wrapper.unmount();
  });

  it("loads data and resets to the first page when page size changes", async () => {
    const loader = vi.fn<ServerPageLoader<number>>()
      .mockResolvedValueOnce(response(1, 50, [1, 2], 102))
      .mockResolvedValueOnce(response(2, 50, [51, 52], 102))
      .mockResolvedValueOnce(response(1, 25, [1], 102));
    let state!: ReturnType<typeof useServerPagination<number>>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useServerPagination(loader);
        return {};
      },
      template: "<div />",
    }));

    await state.load();
    expect(state.items.value).toEqual([1, 2]);
    expect(state.total.value).toBe(102);
    expect(state.totalPages.value).toBe(3);
    expect(state.loading.value).toBe(false);

    state.changePage(2);
    await flushPromises();
    expect(loader).toHaveBeenLastCalledWith(
      { page: 2, page_size: 50, sort: undefined },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.changePageSize(25);
    await flushPromises();
    expect(state.page.value).toBe(1);
    expect(state.pageSize.value).toBe(25);
    expect(loader).toHaveBeenLastCalledWith(
      { page: 1, page_size: 25, sort: undefined },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("resets to the first page and reloads when the sort changes", async () => {
    const loader = vi.fn<ServerPageLoader<number>>()
      .mockResolvedValueOnce(response(1, 50, [1, 2], 102))
      .mockResolvedValueOnce(response(1, 50, [2, 1], 102));
    let state!: ReturnType<typeof useServerPagination<number>>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useServerPagination(loader);
        return {};
      },
      template: "<div />",
    }));

    await state.load();
    state.changePage(2);
    await flushPromises();
    state.changeSort("name", "asc");
    await flushPromises();

    expect(state.page.value).toBe(1);
    expect(state.sort.value).toEqual({ field: "name", direction: "asc" });
    expect(loader).toHaveBeenLastCalledWith(
      { page: 1, page_size: 50, sort: { field: "name", direction: "asc" } },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("ignores a duplicate sort change", async () => {
    const loader = vi.fn<ServerPageLoader<number>>()
      .mockResolvedValue(response(1, 50, [1]));
    let state!: ReturnType<typeof useServerPagination<number>>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useServerPagination(loader);
        return {};
      },
      template: "<div />",
    }));

    await state.load();
    state.changeSort("name", "asc");
    await flushPromises();
    const callsAfterFirstSort = loader.mock.calls.length;
    state.changeSort("name", "asc");
    await flushPromises();
    expect(loader.mock.calls.length).toBe(callsAfterFirstSort);
    wrapper.unmount();
  });

  it("clears the sort back to the server default", async () => {
    const loader = vi.fn<ServerPageLoader<number>>()
      .mockResolvedValue(response(1, 50, [2, 1]));
    let state!: ReturnType<typeof useServerPagination<number>>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useServerPagination(loader);
        return {};
      },
      template: "<div />",
    }));

    await state.load();
    state.changeSort("name", "asc");
    await flushPromises();
    state.clearSort();
    await flushPromises();

    expect(state.sort.value).toBeNull();
    expect(state.page.value).toBe(1);
    expect(loader).toHaveBeenLastCalledWith(
      { page: 1, page_size: 50, sort: undefined },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    state.clearSort();
    await flushPromises();
    const callsAfterClear = loader.mock.calls.length;
    expect(loader.mock.calls.length).toBe(callsAfterClear);
    wrapper.unmount();
  });

  it("ignores a stale response when requests race", async () => {
    const pending: Array<(value: PaginatedResponse<number>) => void> = [];
    const loader = vi.fn<ServerPageLoader<number>>((request) => new Promise((resolve) => {
      pending.push((value) => resolve(value));
      if (request.page === 2) {
        pending[pending.length - 1](response(2, 50, [2], 2));
      }
    }));
    let state!: ReturnType<typeof useServerPagination<number>>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useServerPagination(loader);
        return {};
      },
      template: "<div />",
    }));

    const firstLoad = state.load(1);
    const secondLoad = state.load(2);
    await secondLoad;
    pending[0](response(1, 50, [1], 2));
    await firstLoad;

    expect(state.items.value).toEqual([2]);
    expect(state.page.value).toBe(2);
    wrapper.unmount();
  });
});
