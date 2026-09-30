import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import type { DraftSearchSettings } from "../../utils/settings";
import AppNumberField from "../AppNumberField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";

import SettingsSearchSection from "./SettingsSearchSection.vue";

function createDraft(): DraftSearchSettings {
  return {
    mode: "hybrid",
    rerank_enabled: true,
    rerank_base_url: "https://openrouter.ai/api/v1",
    rerank_model: "cohere/rerank-4-fast",
    candidate_limit: 40,
    timeout_secs: 10,
    vector_weight: 0.55,
    keyword_weight: 0.35,
  };
}

function mountSection() {
  return mount(SettingsSearchSection, {
    props: {
      rerankApiKeyDraft: "",
      rerankToggleModel: { rerank_enabled: true },
      searchDraft: createDraft(),
      searchModeOptions: [
        { label: "Hybrid", value: "hybrid" },
        { label: "Vector only", value: "vector" },
      ],
    },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

describe("SettingsSearchSection", () => {
  it("maps every search field to its settings-schema name", () => {
    const wrapper = mountSection();

    expect(wrapper.get("#search-rerank-base-url").attributes("name")).toBe("search.rerank_base_url");
    expect(wrapper.get("#search-rerank-model").attributes("name")).toBe("search.rerank_model");
    expect(wrapper.get("#search-rerank-api-key").attributes("name")).toBe("search.api_key");

    expect(wrapper.findAllComponents(AppNumberField).map((field) => field.props("name"))).toEqual([
      "search.candidate_limit",
      "search.timeout_secs",
      "search.vector_weight",
      "search.keyword_weight",
    ]);

    const selects = wrapper.findAllComponents(AppSelectField);
    expect(selects).toHaveLength(1);
    expect(selects[0].props("name")).toBe("search.mode");
    expect(wrapper.getComponent(AppToggleGroup).props("name")).toBe("search");
  });

  it("keeps the fusion hint as help text bound to the vector weight field", () => {
    const wrapper = mountSection();
    const describedBy = wrapper.get("#search-vector-weight").attributes("aria-describedby");

    expect(describedBy).toBeTruthy();
    const help = wrapper.get(`#${describedBy}`);
    expect(help.attributes("data-slot")).toBe("help");
    expect(help.text()).toContain("boost margin");
    expect(help.text()).toContain("0.10");
  });

  it("forwards rerank toggle updates as a single-key model", async () => {
    const wrapper = mountSection();

    await wrapper.findAllComponents(AppToggleGroup)[0].vm.$emit("update:modelValue", { rerank_enabled: false });

    expect(wrapper.emitted("update:rerankToggleModel")).toEqual([[{ rerank_enabled: false }]]);
  });
});
