import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import { apiClient, type LibraryProcessingJobResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const getLibraryProcessingJobs = vi.spyOn(apiClient, "getLibraryProcessingJobs");
const retryGroupLibraryFile = vi.spyOn(apiClient, "retryGroupLibraryFile");

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
    getLibraryProcessingJobs.mockReset().mockResolvedValue({ items: [row], page: 1, page_size: 25, total: 1, total_pages: 1 } as never);
    retryGroupLibraryFile.mockReset().mockResolvedValue({ job_id: "new-job-id" } as never);
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
});
