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

function mountSection(draft: DraftDoclingSettings = createDraft()) {
  return mount(SettingsDoclingSection, {
    props: { doclingDraft: draft },
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
});
