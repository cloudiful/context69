import { mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { createRuntimeDraft, type DraftRuntimeSettings } from "../../utils/settings";
import AppNumberField from "../AppNumberField.vue";
import AppToggleField from "../AppToggleField.vue";

import SettingsRuntimeSection from "./SettingsRuntimeSection.vue";

function createDraft(): DraftRuntimeSettings {
  return {
    ...createRuntimeDraft(),
    embedding: {
      base_url: "https://openrouter.ai/api/v1",
      api_key: "",
      model: "text-embedding-3-large",
      dimensions: 3072,
      timeout_secs: 30,
    },
    scheduler: {
      interval_secs: 300,
      run_on_start: true,
      max_concurrency: 4,
      job_id: "context69-sync",
      valkey_url: "redis://valkey:6379/0",
    },
  };
}

function mountSection(draft: DraftRuntimeSettings = createDraft()) {
  return mount(SettingsRuntimeSection, {
    props: {
      qdrantToggleModel: { recreate_on_dimension_mismatch: draft.qdrant.recreate_on_dimension_mismatch },
      runtimeDraft: draft,
      schedulerToggleModel: { run_on_start: draft.scheduler.run_on_start },
      s3Testing: false,
      valkeyTesting: false,
      vectorRebuildStatus: null,
    },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

function toggleByInputId(wrapper: ReturnType<typeof mountSection>, inputId: string) {
  const toggle = wrapper.findAllComponents(AppToggleField).find((item) => item.props("inputId") === inputId);
  expect(toggle).toBeDefined();
  return toggle!;
}

describe("SettingsRuntimeSection", () => {
  it("names runtime fields after their settings-schema paths", () => {
    const wrapper = mountSection();

    expect(wrapper.get("#runtime-embedding-base-url").attributes("name")).toBe("runtime.embedding.base_url");
    expect(wrapper.get("#runtime-qdrant-url").attributes("name")).toBe("runtime.qdrant.url");
    expect(wrapper.get("#runtime-scheduler-valkey-url").attributes("name")).toBe("runtime.scheduler.valkey_url");
    expect(wrapper.get("#runtime-file-library-root").attributes("name")).toBe("runtime.file_library.storage_root");

    expect(wrapper.findAllComponents(AppNumberField).map((field) => field.props("name"))).toEqual([
      "runtime.embedding.dimensions",
      "runtime.embedding.timeout_secs",
      "runtime.scheduler.interval_secs",
      "runtime.scheduler.max_concurrency",
      "runtime.chunking.max_chars",
      "runtime.chunking.overlap_chars",
      "runtime.file_library.max_upload_size_mb",
      "runtime.file_library.max_upload_request_size_mb",
      "runtime.file_library.ingest_concurrency",
      "runtime.file_library.url_import_concurrency",
      "runtime.file_library.url_import_min_interval_ms",
    ]);

    expect(toggleByInputId(wrapper, "runtime-qdrant-recreate").props("name")).toBe(
      "runtime.qdrant.recreate_on_dimension_mismatch",
    );
    expect(toggleByInputId(wrapper, "runtime-scheduler-run-on-start").props("name")).toBe(
      "runtime.scheduler.run_on_start",
    );
    expect(toggleByInputId(wrapper, "runtime-file-library-trusted-proxy").props("name")).toBe(
      "runtime.file_library.trusted_proxy_enabled",
    );
  });

  it("keeps the valkey restart hint and gates the test button on the draft URL", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);
    const describedBy = wrapper.get("#runtime-scheduler-valkey-url").attributes("aria-describedby");

    expect(describedBy).toBeTruthy();
    expect(wrapper.get(`#${describedBy}`).text()).toContain("Restart after saving");

    await wrapper.get('[data-testid="runtime-valkey-test"]').trigger("click");
    expect(wrapper.emitted("test-valkey")).toHaveLength(1);

    draft.scheduler.valkey_url = "   ";
    await nextTick();
    expect(wrapper.get('[data-testid="runtime-valkey-test"]').attributes("disabled")).toBeDefined();

    await wrapper.setProps({ valkeyTesting: true });
    draft.scheduler.valkey_url = "redis://shared-valkey:6379/2";
    await nextTick();
    expect(wrapper.get('[data-testid="runtime-valkey-test"]').attributes("aria-busy")).toBe("true");
  });

  it("reveals the S3 fields only while S3 storage is enabled", async () => {
    const draft = reactive(createDraft());
    draft.file_library.s3_enabled = false;
    const wrapper = mountSection(draft);

    expect(wrapper.find("#runtime-file-library-s3-endpoint").exists()).toBe(false);

    await toggleByInputId(wrapper, "runtime-file-library-s3-enabled").vm.$emit("update:modelValue", true);
    await nextTick();

    expect(draft.file_library.s3_enabled).toBe(true);
    expect(wrapper.get("#runtime-file-library-s3-endpoint").attributes("name")).toBe(
      "runtime.file_library.s3.endpoint",
    );
    expect(wrapper.get("#runtime-file-library-s3-secret-key").attributes("type")).toBe("password");
    expect(toggleByInputId(wrapper, "runtime-file-library-s3-path-style").props("name")).toBe(
      "runtime.file_library.s3.path_style",
    );

    await wrapper.get('[data-testid="runtime-file-library-s3-test"]').trigger("click");
    expect(wrapper.emitted("test-s3")).toHaveLength(1);

    await wrapper.setProps({ s3Testing: true });
    expect(wrapper.get('[data-testid="runtime-file-library-s3-test"]').attributes("aria-busy")).toBe("true");

    await toggleByInputId(wrapper, "runtime-file-library-s3-enabled").vm.$emit("update:modelValue", false);
    await nextTick();
    expect(wrapper.find("#runtime-file-library-s3-endpoint").exists()).toBe(false);
  });

  it("keeps the vector rebuild slot wired to the rebuild event", async () => {
    const wrapper = mountSection();

    expect(wrapper.find('[data-testid="runtime-vector-rebuild"]').exists()).toBe(true);
    await wrapper.get('[data-testid="runtime-vector-rebuild"]').trigger("click");

    expect(wrapper.emitted("rebuild-vector-index")).toHaveLength(1);
  });
});
