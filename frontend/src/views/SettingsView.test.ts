/**
 * The settings view itself: the shared save flow across sections, the page
 * layout, navigation and theme/locale switching, the runtime credential save, the
 * Valkey and Docling connection tests, and the access-token and Docling VLM
 * forms.
 *
 * The runtime-first save ordering and the embedding connection test live in
 * SettingsViewSaveFlow.test.ts; the mount harness and the API spies live in
 * settings-view-test-support.ts.
 */
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  installSettingsViewSpies,
  mountSettingsView,
  surfacedReasons,
  type SettingsApiSpies,
} from "./settings-view-test-support";
import { createTestI18n } from "../test-utils/i18n";
import { LOCALE_STORAGE_KEY } from "../i18n/locale";
import AppNumberField from "../components/AppNumberField.vue";
import AppSelectField from "../components/AppSelectField.vue";
import UPageBody from "@nuxt/ui/components/PageBody.vue";

let apiSpies: SettingsApiSpies;

describe("SettingsView", () => {
  beforeEach(() => {
    apiSpies = installSettingsViewSpies();
  });

  it("loads settings and preserves shared save flow across runtime, docling, and search subpages", async () => {
    const { wrapper, router } = await mountSettingsView("/settings/runtime");

    await vi.waitFor(() => {
      expect(apiSpies.getRuntimeSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.getDoclingSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.getSearchSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.listPersonalAccessTokens).toHaveBeenCalledTimes(1);
      expect(wrapper.find("#runtime-embedding-model").exists()).toBe(true);
    });

    expect(wrapper.get("#runtime-scheduler-valkey-url").attributes("placeholder")).toBe("redis://valkey:6379/0");
    expect(wrapper.get("#runtime-embedding-base-url").element).toBeTruthy();
    expect(wrapper.find("#runtime-embedding-clear-api-key").exists()).toBe(false);
    expect(wrapper.find('[data-testid="runtime-vector-rebuild"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="runtime-file-library-trusted-proxy"]').exists()).toBe(true);
    await wrapper.get('[data-testid="runtime-file-library-trusted-proxy"]').trigger("click");

    await router.push("/settings/docling");
    await flushPromises();
    expect(wrapper.find("#docling-base-url").exists()).toBe(true);
    expect(wrapper.find("#docling-clear-api-key").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Stored key");
    expect(wrapper.text()).not.toContain("No key stored");
    expect(wrapper.get("#docling-vlm-mode").element).toBeTruthy();
    expect(wrapper.get("#docling-picture-description-preset").element).toBeTruthy();
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await wrapper.get("#docling-picture-description-preset").setValue("granite_vision");

    await router.push("/settings/search");
    await flushPromises();
    const vectorWeightField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "search-vector-weight");
    expect(vectorWeightField).toBeDefined();
    await vectorWeightField!.vm.$emit("update:modelValue", 0.65);
    await wrapper.get("#search-rerank-api-key").setValue("rerank-secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledWith(expect.objectContaining({
      embedding: expect.objectContaining({
        base_url: "https://openrouter.ai/api/v1",
        model: "text-embedding-3-large",
      }),
      scheduler: expect.objectContaining({
        valkey_url: "redis://valkey:6379/0",
      }),
      file_library: expect.objectContaining({
        trusted_proxy_enabled: true,
      }),
    }));
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledWith(expect.objectContaining({
      connection: expect.objectContaining({
        base_url: "http://docling.internal:5001",
      }),
      vlm: expect.objectContaining({
        picture_description_preset: "granite_vision",
      }),
    }));
    const updateDoclingCall = apiSpies.updateDoclingSettings.mock.calls.at(-1)?.[0];
    expect(updateDoclingCall?.vlm?.openai_base_url).toBeUndefined();
    expect(updateDoclingCall?.vlm?.api_key).toBeUndefined();
    expect(updateDoclingCall?.vlm?.vlm_pipeline_model).toBeUndefined();
    expect(updateDoclingCall?.vlm?.picture_description_model).toBeUndefined();
    expect(updateDoclingCall?.vlm?.code_formula_model).toBeUndefined();
    expect(apiSpies.updateSearchSettings).toHaveBeenCalledWith(expect.objectContaining({
      mode: "hybrid",
      rerank_enabled: true,
       api_key: { op: "set", value: "rerank-secret" },
      vector_weight: 0.65,
      keyword_weight: 0.35,
    }));
    expect(wrapper.text()).toContain("Settings saved");
  });

  it("keeps long settings sections in a scrollable content region", async () => {
    const { wrapper } = await mountSettingsView("/settings/runtime");

    const scrollRegion = wrapper.get('[data-testid="settings-page-scroll"]');

    expect(scrollRegion.classes()).toEqual(expect.arrayContaining([
      "h-full",
      "min-h-0",
      "grid-rows-[auto_minmax(0,1fr)]",
      "overflow-y-auto",
    ]));
  });

  it("drops the page body theme padding without touching the scroll host", async () => {
    const { wrapper } = await mountSettingsView("/settings/appearance");

    const body = wrapper.findComponent(UPageBody);
    expect(body.exists()).toBe(true);
    expect(body.classes()).toContain("pb-0");
    expect(body.classes()).not.toContain("pb-24");
  });

  it("routes between settings sections from the mobile tab bar", async () => {
    const { router, wrapper } = await mountSettingsView("/settings/appearance");

    const triggers = wrapper.findAll('[data-slot="trigger"]');
    expect(triggers).toHaveLength(5);

    const searchTrigger = triggers.find((trigger) => trigger.text() === "Search");
    expect(searchTrigger).toBeDefined();
    await searchTrigger!.trigger("mousedown");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("settings-search");
  });

  it("switches locale and theme from the settings page", async () => {
    const i18n = createTestI18n("en");
    const { wrapper } = await mountSettingsView("/settings/appearance", i18n);

    const selects = wrapper.findAllComponents(AppSelectField).filter((component) =>
      ["settings-locale-select", "settings-theme-select"].includes(component.props("testId") ?? ""),
    );

    expect(selects).toHaveLength(2);

    await selects[0].vm.$emit("update:modelValue", "zh-CN");
    await selects[1].vm.$emit("update:modelValue", "light");

    expect(i18n.global.locale.value).toBe("zh-CN");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(window.localStorage.getItem("context69.theme")).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("saves direct embedding credentials through the single page save action", async () => {
    const { wrapper } = await mountSettingsView("/settings/runtime");
    await wrapper.get("#runtime-embedding-api-key").setValue("embedding-secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledWith(expect.objectContaining({
      embedding: expect.objectContaining({
        base_url: "https://openrouter.ai/api/v1",
        api_key: "embedding-secret",
      }),
    }));
  });

  it("tests the current Valkey URL without saving settings", async () => {
    const { wrapper } = await mountSettingsView("/settings/runtime");

    await wrapper.get("#runtime-scheduler-valkey-url").setValue(" redis://shared-valkey:6379/2 ");
    await wrapper.get('[data-testid="runtime-valkey-test"]').trigger("click");
    await flushPromises();

    expect(apiSpies.testValkeyConnection).toHaveBeenCalledWith({
      valkey_url: "redis://shared-valkey:6379/2",
    });
    expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
  });

  it("creates and reveals a personal access token", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });

    const { wrapper } = await mountSettingsView("/settings/access-tokens");

    await wrapper.get("#personal-access-token-name").setValue("CLI");
    await wrapper.get('[data-testid="pat-scope-library"]').trigger("click");
    await flushPromises();
    await wrapper.get('[data-testid="personal-access-token-create"]').trigger("click");
    await flushPromises();

    expect(apiSpies.createPersonalAccessToken).toHaveBeenCalledWith({
      name: "CLI",
      scopes: ["search", "library"],
      expires_in_days: 30,
    });
    const secretField = wrapper.get<HTMLTextAreaElement>('[data-testid="personal-access-token-secret"]');
    expect(secretField.element.value).toContain("ctx_pat_secret");
    expect(secretField.element.readOnly).toBe(true);
  });

  it("requires a token name before allowing personal access token creation", async () => {
    const { wrapper } = await mountSettingsView("/settings/access-tokens");

    const createButton = wrapper.get('[data-testid="personal-access-token-create"]');
    expect(createButton.attributes("disabled")).toBeDefined();
    expect(wrapper.text()).not.toContain("Token name is required.");

    await wrapper.get("#personal-access-token-name").setValue("CLI");
    await flushPromises();

    expect(wrapper.get('[data-testid="personal-access-token-create"]').attributes("disabled")).toBeUndefined();
  });

  it("tests the current Docling endpoint without saving settings", async () => {
    const { wrapper } = await mountSettingsView("/settings/docling");

    await wrapper.get("#docling-base-url").setValue(" http://docling.internal:5001 ");
    await wrapper.get('[data-testid="docling-connection-test"]').trigger("click");
    await flushPromises();

    // The probe carries exactly the connection block a save would store, built
    // from the draft on screen, and no VLM block: a Docling connectivity check
    // resolves no credential.
    expect(apiSpies.testDoclingConnection).toHaveBeenCalledWith({
      base_url: "http://docling.internal:5001",
      timeout_secs: 120,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
      max_inflight: 2,
    });
    expect(apiSpies.updateDoclingSettings).not.toHaveBeenCalled();
  });

  it("requires a Docling endpoint before the connection test can run", async () => {
    const { wrapper } = await mountSettingsView("/settings/docling");
    const testButton = wrapper.get('[data-testid="docling-connection-test"]');

    // The loaded draft has an endpoint, so the action starts available; a blank
    // one is not something the probe can check.
    expect(testButton.attributes("disabled")).toBeUndefined();

    await wrapper.get("#docling-base-url").setValue("   ");
    await flushPromises();
    expect(
      wrapper.get('[data-testid="docling-connection-test"]').attributes("disabled"),
    ).toBeDefined();

    await wrapper.get('[data-testid="docling-connection-test"]').trigger("click");
    await flushPromises();
    expect(apiSpies.testDoclingConnection).not.toHaveBeenCalled();
  });

  it("surfaces a failed Docling connection test without saving settings", async () => {
    apiSpies.testDoclingConnection.mockRejectedValueOnce(
      Object.assign(new Error("the Docling endpoint answered 503"), { name: "ApiError" }),
    );

    const { wrapper } = await mountSettingsView("/settings/docling");
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await wrapper.get('[data-testid="docling-connection-test"]').trigger("click");
    await flushPromises();

    expect(apiSpies.testDoclingConnection).toHaveBeenCalledTimes(1);
    expect(surfacedReasons()).toContain("the Docling endpoint answered 503");
    expect(apiSpies.updateDoclingSettings).not.toHaveBeenCalled();
  });

  it("switches the docling VLM form between disabled, preset, and custom modes", async () => {
    const { wrapper } = await mountSettingsView("/settings/docling");
    await flushPromises();

    const modeSelect = wrapper.findAllComponents(AppSelectField).find((component) =>
      component.props("inputId") === "docling-vlm-mode",
    );
    expect(modeSelect).toBeDefined();
    expect(modeSelect!.props("modelValue")).toBe("preset");

    // preset mode (loaded by default since the response carries a preset) shows only the preset input
    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(true);
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
    expect(wrapper.find("#docling-api-key").exists()).toBe(false);

    await modeSelect!.vm.$emit("update:modelValue", "custom");
    await flushPromises();
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(true);
    expect(wrapper.find("#docling-api-key").exists()).toBe(true);
    expect(wrapper.find("#docling-vlm-pipeline-model").exists()).toBe(true);
    expect(wrapper.find("#docling-picture-description-model").exists()).toBe(true);
    expect(wrapper.find("#docling-code-formula-model").exists()).toBe(true);
    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);

    await modeSelect!.vm.$emit("update:modelValue", "disabled");
    await flushPromises();
    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
    expect(wrapper.find("#docling-api-key").exists()).toBe(false);
    expect(wrapper.find("#docling-code-formula-model").exists()).toBe(false);
  });

  it("infers disabled mode when no preset or legacy VLM fields are stored", async () => {
    apiSpies.getDoclingSettings.mockResolvedValueOnce({
      configured: true,
      source: "database",
      connection: {
        base_url: "http://docling:5001",
        timeout_secs: 120,
        poll_interval_secs: 2,
        task_timeout_secs: 600,
      },
      vlm: {
        openai_base_url: null,
        has_api_key: false,
        vlm_pipeline_model: null,
        picture_description_model: null,
        code_formula_model: null,
        picture_description_preset: null,
      },
    } as never);

    const { wrapper } = await mountSettingsView("/settings/docling");
    await flushPromises();

    const modeSelect = wrapper.findAllComponents(AppSelectField).find((component) =>
      component.props("inputId") === "docling-vlm-mode",
    );
    expect(modeSelect).toBeDefined();
    expect(modeSelect!.props("modelValue")).toBe("disabled");
    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);

    // Touch the connection URL so the save flow actually fires for a
    // disabled-mode draft where every VLM field is already a no-op.
    await wrapper.get("#docling-base-url").setValue("http://docling:5002");

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const updateDoclingCall = apiSpies.updateDoclingSettings.mock.calls.at(-1)?.[0];
    expect(updateDoclingCall?.vlm).toEqual({
      openai_base_url: undefined,
      api_key: undefined,
      vlm_pipeline_model: undefined,
      picture_description_model: undefined,
      picture_description_preset: undefined,
      code_formula_model: undefined,
    });
  });

  it("switches to custom mode and submits the legacy bundle without the preset", async () => {
    const { wrapper } = await mountSettingsView("/settings/docling");
    await flushPromises();

    const modeSelect = wrapper.findAllComponents(AppSelectField).find((component) =>
      component.props("inputId") === "docling-vlm-mode",
    );
    expect(modeSelect).toBeDefined();
    await modeSelect!.vm.$emit("update:modelValue", "custom");
    await flushPromises();

    await wrapper.get("#docling-openai-base-url").setValue("https://openrouter.ai/api/v1");
    await wrapper.get("#docling-api-key").setValue("sk-new");
    await wrapper.get("#docling-vlm-pipeline-model").setValue("gemini-3-flash");
    await wrapper.get("#docling-picture-description-model").setValue("gpt-4o-mini");
    await wrapper.get("#docling-code-formula-model").setValue("gpt-4o-mini");

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const updateDoclingCall = apiSpies.updateDoclingSettings.mock.calls.at(-1)?.[0];
    expect(updateDoclingCall?.vlm).toEqual({
      openai_base_url: "https://openrouter.ai/api/v1",
      api_key: "sk-new",
      vlm_pipeline_model: "gemini-3-flash",
      picture_description_model: "gpt-4o-mini",
      picture_description_preset: undefined,
      code_formula_model: "gpt-4o-mini",
    });
  });
});