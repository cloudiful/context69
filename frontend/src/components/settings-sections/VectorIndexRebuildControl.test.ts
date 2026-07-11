import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testPrimeVuePlugin } from "../../test-utils/primevue";

import VectorIndexRebuildControl from "./VectorIndexRebuildControl.vue";

describe("VectorIndexRebuildControl", () => {
  it("starts a rebuild and disables repeated actions while running", async () => {
    const wrapper = mount(VectorIndexRebuildControl, {
      props: { status: { state: "idle", processed_chunks: 0, total_chunks: 0 } },
      global: { plugins: [testPrimeVuePlugin, createTestI18n("zh-CN")] },
    });

    await wrapper.get('[data-testid="runtime-vector-rebuild"]').trigger("click");
    expect(wrapper.emitted("rebuild")).toHaveLength(1);

    await wrapper.setProps({
      status: { state: "running", processed_chunks: 64, total_chunks: 128 },
    });
    expect(wrapper.get('[data-testid="runtime-vector-rebuild"]').attributes("disabled")).toBeDefined();
    expect(wrapper.text()).toContain("64 / 128");
  });
});
