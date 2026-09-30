import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import UButton from "@nuxt/ui/components/Button.vue";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import AppToggleField from "./AppToggleField.vue";

import GroupTranslationSettingsSection from "./GroupTranslationSettingsSection.vue";

const getSettings = vi.spyOn(apiClient, "getGroupTranslationSettings");
const updateSettings = vi.spyOn(apiClient, "updateGroupTranslationSettings");

const settingsResponse = {
  enabled: true,
  default_target_locales: ["zh-CN", "ja-JP"],
  source_locale: "en-US",
  glossary: [{ source: "stock", target: "股票" }],
  queued_count: 1,
  running_count: 2,
  succeeded_count: 3,
  failed_count: 4,
};

function mountSection(canManage = true) {
  return mount(GroupTranslationSettingsSection, {
    props: { groupPath: "stock", canManage },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

function inputValue(wrapper: ReturnType<typeof mountSection>, selector: string) {
  return (wrapper.get(selector).element as HTMLInputElement | HTMLTextAreaElement).value;
}

describe("GroupTranslationSettingsSection", () => {
  beforeEach(() => {
    getSettings.mockReset();
    updateSettings.mockReset();
    getSettings.mockResolvedValue(settingsResponse as never);
    updateSettings.mockResolvedValue(settingsResponse as never);
  });

  it("loads the group translation defaults into named fields", async () => {
    const wrapper = mountSection();
    await flushPromises();

    expect(inputValue(wrapper, "#group-translation-targets")).toBe("zh-CN, ja-JP");
    expect(wrapper.get("#group-translation-targets").attributes("name")).toBe(
      "group_translation.default_target_locales",
    );
    expect(inputValue(wrapper, "#group-translation-source")).toBe("en-US");
    expect(wrapper.get("#group-translation-source").attributes("name")).toBe("group_translation.source_locale");
    expect(inputValue(wrapper, "#group-translation-glossary")).toBe("stock = 股票");
    expect(wrapper.get("#group-translation-glossary").attributes("name")).toBe("group_translation.glossary");
    expect(wrapper.getComponent(AppToggleField).props("name")).toBe("group_translation.enabled");
    expect(wrapper.text()).toContain("1 queued");
  });

  it("saves the parsed glossary through the independent update call", async () => {
    const wrapper = mountSection();
    await flushPromises();

    await wrapper.get("#group-translation-targets").setValue("zh-CN, ja-JP, ko-KR");
    await wrapper.get("#group-translation-glossary").setValue("stock = 股票\nbond=债券");
    await wrapper.findComponent(UButton).trigger("click");
    await flushPromises();

    expect(updateSettings).toHaveBeenCalledWith("stock", {
      enabled: true,
      default_target_locales: ["zh-CN", "ja-JP", "ko-KR"],
      source_locale: "en-US",
      glossary: [
        { source: "stock", target: "股票" },
        { source: "bond", target: "债券" },
      ],
    });
    expect(getSettings).toHaveBeenCalledTimes(2);
  });

  it("rejects a glossary line without a target instead of saving", async () => {
    const wrapper = mountSection();
    await flushPromises();

    await wrapper.get("#group-translation-glossary").setValue("stock");
    await wrapper.findComponent(UButton).trigger("click");
    await flushPromises();

    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("disables editing and hides save for members without manage rights", async () => {
    const wrapper = mountSection(false);
    await flushPromises();

    expect(wrapper.get("#group-translation-targets").attributes("disabled")).toBeDefined();
    expect(wrapper.get("#group-translation-glossary").attributes("disabled")).toBeDefined();
    expect(wrapper.getComponent(AppToggleField).props("disabled")).toBe(true);
    expect(wrapper.findComponent(UButton).exists()).toBe(false);
  });
});
