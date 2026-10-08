import { mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { createDoclingDraft, type DraftDoclingSettings } from "../../utils/settings";
import AppSelectField from "../AppSelectField.vue";
import AppNumberField from "../AppNumberField.vue";

import SettingsDoclingSection from "./SettingsDoclingSection.vue";

function createDraft(): DraftDoclingSettings {
  return {
    ...createDoclingDraft(),
    connection: {
      base_url: "http://docling:5001",
      timeout_secs: 120,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
      max_inflight: 2,
    },
  };
}

function mountSection(draft: DraftDoclingSettings = createDraft(), doclingTesting = false) {
  return mount(SettingsDoclingSection, {
    props: { doclingDraft: draft, doclingTesting },
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n("en")],
      stubs: {
        UTooltip: {
          template: "<div><slot /><div class='tooltip-stub'>{{ $attrs.text }}</div></div>",
        },
      },
    },
  });
}

describe("SettingsDoclingSection", () => {
  it("names every docling field after its settings-schema path", () => {
    const wrapper = mountSection();

    expect(wrapper.get("#docling-base-url").attributes("name")).toBe("docling.connection.base_url");
    expect(wrapper.findAllComponents(AppNumberField).map((field) => field.props("name"))).toEqual([
      "docling.connection.timeout_secs",
      "docling.connection.poll_interval_secs",
      "docling.connection.task_timeout_secs",
      "docling.connection.max_inflight",
    ]);

    const modeSelect = wrapper.findAllComponents(AppSelectField)[0];
    expect(modeSelect.props("inputId")).toBe("docling-vlm-mode");
    expect(modeSelect.props("name")).toBe("docling.vlm_mode");
  });

  it("renders the preset field only in preset mode", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);

    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);

    draft.vlm_mode = "preset";
    await nextTick();

    expect(wrapper.get("#docling-picture-description-preset").attributes("name")).toBe(
      "docling.vlm.picture_description_preset",
    );
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
  });

  it("renders the legacy bundle with schema names only in custom mode", async () => {
    const draft = reactive(createDraft());
    draft.vlm_mode = "custom";
    const wrapper = mountSection(draft);

    expect(wrapper.get("#docling-openai-base-url").attributes("name")).toBe("docling.vlm.openai_base_url");
    expect(wrapper.get("#docling-api-key").attributes("name")).toBe("docling.vlm.api_key");
    expect(wrapper.get("#docling-vlm-pipeline-model").attributes("name")).toBe("docling.vlm.vlm_pipeline_model");
    expect(wrapper.get("#docling-picture-description-model").attributes("name")).toBe(
      "docling.vlm.picture_description_model",
    );
    expect(wrapper.get("#docling-code-formula-model").attributes("name")).toBe("docling.vlm.code_formula_model");
    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);

    draft.vlm_mode = "disabled";
    await nextTick();

    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
    expect(wrapper.find("#docling-api-key").exists()).toBe(false);
    expect(wrapper.find("#docling-picture-description-model").exists()).toBe(false);
  });

  it("binds the max in-flight helper text to the field", () => {
    const wrapper = mountSection();
    const describedBy = wrapper.get("#docling-max-inflight").attributes("aria-describedby");

    expect(describedBy).toBeTruthy();
    const help = wrapper.get(`#${describedBy}`);
    expect(help.attributes("data-slot")).toBe("help");
    expect(help.text()).toContain("Mac mini");
  });

  it("leaves the VLM mode tooltip describing the active mode", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);

    expect(wrapper.get(".tooltip-stub").text()).toContain("Turn Docling VLM off");

    draft.vlm_mode = "preset";
    await nextTick();
    expect(wrapper.get(".tooltip-stub").text()).toContain("picture-description preset only");

    draft.vlm_mode = "custom";
    await nextTick();
    expect(wrapper.get(".tooltip-stub").text()).toContain("OpenAI-compatible base URL");
    expect(wrapper.get('[data-testid="docling-vlm-mode-info"]').attributes("aria-label")).toBe("VLM mode details");
  });

  it("puts the test action in the connection block title row", () => {
    const wrapper = mountSection();
    const connection = wrapper.get("#settings-connection");

    // The action belongs to the connection block's title row, beside its title,
    // rather than in the block body: the title row is the first child of the
    // block element, the same place the other settings test actions sit.
    const titleRow = connection.element.firstElementChild!;
    expect(titleRow.textContent).toContain("Connection");
    expect(titleRow.textContent).toContain("Test Connection");
    expect(titleRow.querySelector('[data-testid="docling-connection-test"]')).not.toBeNull();
    expect(wrapper.get('[data-testid="docling-connection-test"]').text()).toBe("Test Connection");

    // It is the connection block's action, not the VLM block's.
    expect(wrapper.get("#settings-vlm").find('[data-testid="docling-connection-test"]').exists()).toBe(false);
  });

  it("disables the test action until the endpoint has a value", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);

    draft.connection.base_url = "   ";
    await nextTick();
    expect(wrapper.get('[data-testid="docling-connection-test"]').attributes("disabled")).toBeDefined();

    draft.connection.base_url = "http://docling.internal:5001";
    await nextTick();
    expect(
      wrapper.get('[data-testid="docling-connection-test"]').attributes("disabled"),
    ).toBeUndefined();
  });

  it("disables the test action while a probe is already running", () => {
    const wrapper = mountSection(createDraft(), true);

    const button = wrapper.get('[data-testid="docling-connection-test"]');
    expect(button.attributes("disabled")).toBeDefined();
    expect(button.attributes("aria-busy")).toBe("true");
  });

  it("asks for a probe and never for a save", async () => {
    const wrapper = mountSection();

    await wrapper.get('[data-testid="docling-connection-test"]').trigger("click");

    // The section owns only the intent: it emits the probe request and exposes
    // no way to persist the draft, so a test cannot be mistaken for a save.
    expect(wrapper.emitted("test-docling")).toHaveLength(1);
    expect(wrapper.emitted()).not.toHaveProperty("update:doclingDraft");
  });
});
