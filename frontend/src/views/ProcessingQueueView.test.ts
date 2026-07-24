import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import { apiClient, type LibraryProcessingJobResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const getLibraryProcessingJobs = vi.spyOn(apiClient, "getLibraryProcessingJobs");
const retryGroupLibraryFile = vi.spyOn(apiClient, "retryGroupLibraryFile");
const retryFailedLibraryProcessingJobs = vi.spyOn(apiClient, "retryFailedLibraryProcessingJobs");
const cleanupStuckLibraryProcessingJobs = vi.spyOn(apiClient, "cleanupStuckLibraryProcessingJobs");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
const addToast = vi.fn();
const useToast = vi.spyOn(nuxtUiComposables, "useToast");

let confirmResult = true;

const row: LibraryProcessingJobResponse = {
  can_retry: true,
  created_at: "2026-07-20T00:00:00Z",
  error_message: "Qdrant unavailable",
  failure_stage: "indexing",
  file_id: "file-id",
  filename: "report.pdf",
  finished_at: "2026-07-20T00:02:00Z",
  group_key: "research",
  group_path: "research",
  job_id: "job-id",
  kind: "ingest",
  source_url: null,
  started_at: "2026-07-20T00:01:00Z",
  status: "failed",
  updated_at: "2026-07-20T00:02:00Z",
  visibility: "private",
};

describe("ProcessingQueueView", () => {
  beforeEach(() => {
    getLibraryProcessingJobs.mockReset().mockResolvedValue({
      items: [row],
      pagination: { page: 1, page_size: 25, total: 1, total_pages: 1 },
      summary: {
        can_manage: false,
        cleanupable_stuck_count: 0,
        failed_count: 1,
        pending_count: 0,
        retryable_failed_count: 0,
        running_count: 0,
        stuck_count: 0,
      },
    } as never);
    retryGroupLibraryFile.mockReset().mockResolvedValue({ job_id: "new-job-id" } as never);
    retryFailedLibraryProcessingJobs.mockReset().mockResolvedValue({ accepted: 1, skipped: 0 } as never);
    cleanupStuckLibraryProcessingJobs.mockReset().mockResolvedValue({ accepted: 1, skipped: 0 } as never);
    confirmResult = true;
    useOverlay.mockReset().mockReturnValue({
      create: () => ({ open: async () => confirmResult }),
    } as never);
    addToast.mockReset();
    useToast.mockReset().mockReturnValue({ add: addToast } as never);
  });

  it("shows failure stage and retries the whole file without reloading the queue", async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
    });
    await router.push("/processing-queue");
    await router.isReady();
    const wrapper = mount(ProcessingQueueView, {
      global: { plugins: [testNuxtUiPlugin, createTestI18n("en"), router] },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("Indexing");
    expect(wrapper.text()).toContain("Qdrant unavailable");

    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry file"));
    expect(retryButton).toBeDefined();
    await retryButton!.trigger("click");
    await flushPromises();

    expect(retryGroupLibraryFile).toHaveBeenCalledWith("research", "file-id");
    expect(getLibraryProcessingJobs).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain("Pending");
    wrapper.unmount();
  });

  it("does not call a bulk endpoint when confirmation is cancelled", async () => {
    confirmResult = false;
    getLibraryProcessingJobs.mockResolvedValue({
      items: [row],
      pagination: { page: 1, page_size: 25, total: 1, total_pages: 1 },
      summary: {
        can_manage: true,
        cleanupable_stuck_count: 1,
        failed_count: 1,
        pending_count: 1,
        retryable_failed_count: 1,
        running_count: 0,
        stuck_count: 1,
      },
    } as never);
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
    });
    await router.push("/processing-queue");
    await router.isReady();
    const wrapper = mount(ProcessingQueueView, {
      global: { plugins: [testNuxtUiPlugin, createTestI18n("en"), router] },
    });
    await flushPromises();

    const retryAllButton = wrapper.findAll("button").find((button) => button.text().includes("Retry all failed"));
    expect(retryAllButton).toBeDefined();
    await retryAllButton!.trigger("click");
    await flushPromises();

    expect(retryFailedLibraryProcessingJobs).not.toHaveBeenCalled();
    expect(getLibraryProcessingJobs).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("confirms bulk retry, refreshes once, and reports accepted/skipped counts", async () => {
    getLibraryProcessingJobs
      .mockResolvedValueOnce({
        items: [row],
        pagination: { page: 1, page_size: 25, total: 1, total_pages: 1 },
        summary: {
          can_manage: true,
          cleanupable_stuck_count: 0,
          failed_count: 1,
          pending_count: 0,
          retryable_failed_count: 1,
          running_count: 0,
          stuck_count: 0,
        },
      } as never)
      .mockResolvedValueOnce({
        items: [],
        pagination: { page: 1, page_size: 25, total: 0, total_pages: 0 },
        summary: {
          can_manage: true,
          cleanupable_stuck_count: 0,
          failed_count: 0,
          pending_count: 1,
          retryable_failed_count: 0,
          running_count: 0,
          stuck_count: 0,
        },
      } as never);
    retryFailedLibraryProcessingJobs.mockResolvedValue({ accepted: 3, skipped: 2 } as never);
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
    });
    await router.push("/processing-queue");
    await router.isReady();
    const wrapper = mount(ProcessingQueueView, {
      global: { plugins: [testNuxtUiPlugin, createTestI18n("en"), router] },
    });
    await flushPromises();

    const retryAllButton = wrapper.findAll("button").find((button) => button.text().includes("Retry all failed"));
    expect(retryAllButton).toBeDefined();
    await retryAllButton!.trigger("click");
    await flushPromises();

    expect(retryFailedLibraryProcessingJobs).toHaveBeenCalledOnce();
    expect(getLibraryProcessingJobs).toHaveBeenCalledTimes(2);
    expect(addToast).toHaveBeenCalledWith(expect.objectContaining({
      description: "Accepted: 3; Skipped: 2",
    }));
    wrapper.unmount();
  });
});
