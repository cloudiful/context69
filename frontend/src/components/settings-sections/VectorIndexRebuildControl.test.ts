import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";

import VectorIndexRebuildControl from "./VectorIndexRebuildControl.vue";

describe("VectorIndexRebuildControl", () => {
  it("starts a rebuild and disables repeated actions while running", async () => {
    const wrapper = mount(VectorIndexRebuildControl, {
      props: { status: null },
      global: { plugins: [testNuxtUiPlugin, createTestI18n("zh-CN")] },
    });

    await wrapper.get('[data-testid="runtime-vector-rebuild"]').trigger("click");
    expect(wrapper.emitted("rebuild")).toHaveLength(1);

    await wrapper.setProps({
      status: {
        task_id: "00000000-0000-0000-0000-000000000001",
        kind: "vector_rebuild",
        status: "running",
        group_path: null,
        source_key: null,
        stage: "indexing",
        waiting_reason: null,
        dependency_key: null,
        progress: { total: 1, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
        failure_stage: null,
        error_summary: null,
        eta_seconds: null,
        created_at: "2026-08-02T00:00:00Z",
        started_at: "2026-08-02T00:00:00Z",
        finished_at: null,
        updated_at: "2026-08-02T00:00:00Z",
      },
    });
    expect(wrapper.get('[data-testid="runtime-vector-rebuild"]').attributes("disabled")).toBeDefined();
    expect(wrapper.text()).toContain("运行中");
  });
});
