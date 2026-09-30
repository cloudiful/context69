import { DOMWrapper, flushPromises, mount } from "@vue/test-utils";
import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../../services/api";
import { settingsPageStateKey } from "../../composables/settings-page-context";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import AppNumberField from "../../components/AppNumberField.vue";
import AppSelectField from "../../components/AppSelectField.vue";
import AppTextField from "../../components/AppTextField.vue";

import SettingsTranslationPage from "./SettingsTranslationPage.vue";

const providerPage = {
  items: [
    {
      provider: "llm" as const,
      priority: 0,
      enabled: true,
      endpoint: "https://openrouter.ai/api/v1",
      has_api_key: true,
      model: "gemini-3-flash",
      llm_api_kind: "openai_responses" as const,
      deepl_plan: null,
      monthly_character_limit: null,
      current_month_characters: 1200,
    },
    {
      provider: "deepl" as const,
      priority: 1,
      enabled: false,
      endpoint: "https://api.deepl.com",
      has_api_key: false,
      model: null,
      llm_api_kind: null,
      deepl_plan: "free" as const,
      monthly_character_limit: 500000,
      current_month_characters: 0,
    },
  ],
  pagination: { page: 1, page_size: 50, total: 2, total_pages: 1 },
};

function mountPage() {
  const providers = ref(providerPage.items.map((provider) => ({ ...provider })));
  const wrapper = mount(SettingsTranslationPage, {
    attachTo: document.body,
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n("en")],
      provide: { [settingsPageStateKey]: { translationProviders: providers } },
    },
  });
  return { wrapper, providers };
}

type PageWrapper = ReturnType<typeof mountPage>["wrapper"];

async function openEdit(wrapper: PageWrapper, index: number): Promise<DOMWrapper<HTMLElement>> {
  const editButtons = wrapper.findAll("button").filter((button) => button.attributes("aria-label") === "Edit");
  await editButtons[index].trigger("click");

  const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
  expect(dialog).not.toBeNull();
  return new DOMWrapper(dialog!);
}

describe("SettingsTranslationPage", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
    vi.spyOn(apiClient, "listTranslationProviders").mockResolvedValue(providerPage as never);
  });

  it("fetches the provider page and lists the returned providers", async () => {
    const { wrapper } = mountPage();
    await flushPromises();

    expect(apiClient.listTranslationProviders).toHaveBeenCalledWith({ page: 1, pageSize: 50 });
    expect(wrapper.text()).toContain("LLM");
    expect(wrapper.text()).toContain("DeepL");
  });

  it("names the llm dialog fields and keeps the llm explanation as the dialog description", async () => {
    const { wrapper } = mountPage();
    await flushPromises();

    const modal = await openEdit(wrapper, 0);

    expect(modal.get('[data-slot="title"]').text()).toBe("LLM");
    expect(modal.get('[data-slot="description"]').text()).toContain("shared by translation");

    expect(wrapper.findAllComponents(AppTextField).map((field) => field.props("name"))).toEqual([
      "translation.llm.endpoint",
      "translation.llm.api_key",
      "translation.llm.model",
    ]);
    expect(wrapper.findAllComponents(AppSelectField).map((field) => field.props("name"))).toEqual([
      "translation.llm.llm_api_kind",
    ]);
    expect(wrapper.findAllComponents(AppNumberField).map((field) => field.props("name"))).toEqual([
      "translation.llm.monthly_character_limit",
    ]);
  });

  it("shows the deepl plan and lifetime quota without the llm-only fields", async () => {
    const { wrapper } = mountPage();
    await flushPromises();

    const modal = await openEdit(wrapper, 1);

    expect(modal.get('[data-slot="title"]').text()).toBe("DeepL");
    expect(modal.find('[data-slot="description"]').exists()).toBe(false);
    expect(wrapper.findAllComponents(AppSelectField).map((field) => field.props("name"))).toEqual([
      "translation.deepl.deepl_plan",
    ]);
    expect(wrapper.findAllComponents(AppTextField).map((field) => field.props("name"))).toEqual([
      "translation.deepl.endpoint",
      "translation.deepl.api_key",
    ]);
    expect(wrapper.findAllComponents(AppNumberField).map((field) => field.props("name"))).toEqual([
      "translation.deepl.monthly_character_limit",
    ]);
  });

  it("applies the dialog edit to the provider list without touching the global save flow", async () => {
    const { wrapper, providers } = mountPage();
    await flushPromises();

    const modal = await openEdit(wrapper, 0);
    await modal.get("#translation-llm-endpoint").setValue("https://llm.internal/v1");

    const confirm = modal.findAll("button").find((button) => button.text() === "Confirm");
    expect(confirm).toBeDefined();
    await confirm!.trigger("click");

    expect(providers.value[0].endpoint).toBe("https://llm.internal/v1");
    expect(providers.value[1].endpoint).toBe("https://api.deepl.com");
  });
});
