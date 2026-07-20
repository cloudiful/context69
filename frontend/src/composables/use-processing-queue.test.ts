import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type LibraryProcessingJobResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { useProcessingQueue } from "./use-processing-queue";

const getLibraryProcessingJobs = vi.spyOn(apiClient, "getLibraryProcessingJobs");
const retryGroupLibraryFile = vi.spyOn(apiClient, "retryGroupLibraryFile");
const retryGroupLibraryUrlImportJob = vi.spyOn(apiClient, "retryGroupLibraryUrlImportJob");

const failedIngest: LibraryProcessingJobResponse = {
  can_retry: true,
  created_at: "2026-07-20T00:00:00Z",
  error_message: "Embedding failed",
  failure_stage: "embedding",
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

function page(items: LibraryProcessingJobResponse[] = [failedIngest]) {
  return { items, page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 };
}

describe("useProcessingQueue", () => {
  beforeEach(() => {
    getLibraryProcessingJobs.mockReset().mockResolvedValue(page() as never);
    retryGroupLibraryFile.mockReset();
    retryGroupLibraryUrlImportJob.mockReset();
  });

  function mountState() {
    let state!: ReturnType<typeof useProcessingQueue>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProcessingQueue({ t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
    return { state, wrapper };
  }

  it("loads once on mount and applies filters, search, and pagination explicitly", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(getLibraryProcessingJobs).toHaveBeenCalledOnce();
    expect(getLibraryProcessingJobs).toHaveBeenLastCalledWith(
      { page: 1, pageSize: 25, query: "", status: null, failureStage: null },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.setStatusFilter("failed");
    await flushPromises();
    expect(getLibraryProcessingJobs).toHaveBeenLastCalledWith(
      { page: 1, pageSize: 25, query: "", status: "failed", failureStage: null },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.searchInput.value = "embedding";
    state.submitSearch();
    await flushPromises();
    expect(getLibraryProcessingJobs).toHaveBeenLastCalledWith(
      { page: 1, pageSize: 25, query: "embedding", status: "failed", failureStage: null },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.changePage(2);
    await flushPromises();
    expect(getLibraryProcessingJobs).toHaveBeenLastCalledWith(
      { page: 2, pageSize: 25, query: "embedding", status: "failed", failureStage: null },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("updates only the retried ingest row locally", async () => {
    retryGroupLibraryFile.mockResolvedValue({ job_id: "new-job-id" } as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.retryJob(failedIngest);

    expect(retryGroupLibraryFile).toHaveBeenCalledWith("research", "file-id");
    expect(getLibraryProcessingJobs).toHaveBeenCalledOnce();
    expect(state.items.value[0]).toMatchObject({
      job_id: "new-job-id",
      status: "pending",
      failure_stage: null,
      error_message: null,
    });
    wrapper.unmount();
  });

  it("uses the URL import retry endpoint for URL rows", async () => {
    const urlItem: LibraryProcessingJobResponse = {
      ...failedIngest,
      file_id: null,
      filename: null,
      kind: "url_import",
      job_id: "import-job-id",
      source_url: "https://example.test/report.pdf",
    };
    getLibraryProcessingJobs.mockResolvedValue(page([urlItem]) as never);
    retryGroupLibraryUrlImportJob.mockResolvedValue({ import_job_id: "new-import-job-id" } as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.retryJob(urlItem);

    expect(retryGroupLibraryUrlImportJob).toHaveBeenCalledWith("research", "import-job-id");
    expect(state.items.value[0]).toMatchObject({ job_id: "new-import-job-id", status: "pending" });
    wrapper.unmount();
  });
});
