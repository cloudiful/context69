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
      embeddingTesting: false,
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

  it("wires the embedding test to its title-row action", async () => {
    const wrapper = mountSection();
    const button = wrapper.get('[data-testid="runtime-embedding-test"]');

    expect(button.text()).toContain("Test Connection");
    await button.trigger("click");
    expect(wrapper.emitted("test-embedding")).toHaveLength(1);

    await wrapper.setProps({ embeddingTesting: true });
    expect(wrapper.get('[data-testid="runtime-embedding-test"]').attributes("aria-busy")).toBe("true");
  });

  it("places the connection tests on their section title rows", async () => {
    const wrapper = mountSection();

    expect(wrapper.get("#settings-embedding").find('[data-testid="runtime-embedding-test"]').exists()).toBe(true);
    expect(wrapper.get("#settings-scheduler").find('[data-testid="runtime-valkey-test"]').exists()).toBe(true);

    const draft = reactive(createDraft());
    draft.file_library.s3_enabled = true;
    const withS3 = mountSection(draft);
    expect(withS3.get("#settings-file-library").find('[data-testid="runtime-file-library-s3-test"]').exists()).toBe(true);
  });

  it("leaves the vector rebuild control in the qdrant body, not on a title row", () => {
    const wrapper = mountSection();
    const qdrant = wrapper.get("#settings-qdrant");
    const titleRow = qdrant.element.firstElementChild!;

    expect(titleRow.querySelector('[data-testid="runtime-vector-rebuild"]')).toBeNull();
    expect(qdrant.find('[data-testid="runtime-vector-rebuild"]').exists()).toBe(true);
  });
});

const CONNECTION_TESTS = [
  { testId: "runtime-embedding-test", section: "#settings-embedding", prop: "embeddingTesting", event: "test-embedding" },
  { testId: "runtime-valkey-test", section: "#settings-scheduler", prop: "valkeyTesting", event: "test-valkey" },
  { testId: "runtime-file-library-s3-test", section: "#settings-file-library", prop: "s3Testing", event: "test-s3" },
] as const;

describe("the runtime connection tests", () => {
  it("are rendered by one shared presentation on their own section title row", () => {
    const draft = reactive(createDraft());
    draft.file_library.s3_enabled = true;
    const wrapper = mountSection(draft);

    const presentations = CONNECTION_TESTS.map(({ testId, section }) => {
      const button = wrapper.get(`${section} [data-testid="${testId}"]`);
      // One style for all three, so the layout cannot drift per section.
      return {
        classes: button.attributes("class"),
        icons: button.findAll("svg").length,
        label: button.text(),
        busy: button.attributes("aria-busy"),
        disabled: button.attributes("disabled"),
      };
    });

    expect(new Set(presentations.map((item) => item.classes)).size).toBe(1);
    expect(new Set(presentations.map((item) => item.label)).size).toBe(1);
    for (const presentation of presentations) {
      expect(presentation.icons).toBe(1);
      expect(presentation.busy).toBe("false");
      expect(presentation.disabled).toBeUndefined();
    }
  });

  it("all switch to one busy state and back", async () => {
    const draft = reactive(createDraft());
    draft.file_library.s3_enabled = true;
    const wrapper = mountSection(draft);
    const label = wrapper.get('[data-testid="runtime-embedding-test"]').text();

    for (const { testId, prop, event } of CONNECTION_TESTS) {
      await wrapper.setProps({ [prop]: true });

      const running = wrapper.get(`[data-testid="${testId}"]`);
      expect(running.attributes("aria-busy"), testId).toBe("true");
      expect(running.attributes("disabled"), testId).toBeDefined();
      // The spinner replaces the resting icon in place, and the label stays put.
      expect(running.findAll("svg").length, testId).toBe(1);
      expect(running.text(), testId).toBe(label);

      await wrapper.setProps({ [prop]: false });
      const idle = wrapper.get(`[data-testid="${testId}"]`);
      expect(idle.attributes("aria-busy"), testId).toBe("false");
      expect(idle.attributes("disabled"), testId).toBeUndefined();
      expect(idle.attributes("class"), testId).toBe(running.attributes("class"));

      await idle.trigger("click");
      expect(wrapper.emitted(event) ?? [], event).toHaveLength(1);
    }
  });
});
