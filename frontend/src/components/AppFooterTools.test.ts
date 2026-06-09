import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";
import SelectButton from "primevue/selectbutton";

import { createAppI18n } from "../i18n";
import { LOCALE_STORAGE_KEY } from "../i18n/locale";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { installMockStorage } from "../test-utils/storage";
import { useUiPreferences } from "../composables/use-ui-preferences";
import AppFooterTools from "./AppFooterTools.vue";

describe("AppFooterTools", () => {
  beforeEach(() => {
    installMockStorage();
    const preferences = useUiPreferences();
    preferences.setTheme("dark");
    if (preferences.state.sidebarCollapsed) {
      preferences.toggleSidebar();
    }
  });

  it("switches locale and theme and persists both", async () => {
    const i18n = createAppI18n("en");
    const wrapper = mount(AppFooterTools, {
      global: {
        plugins: [testPrimeVuePlugin, i18n],
      },
    });

    const selectButtons = wrapper.findAllComponents(SelectButton);
    expect(selectButtons).toHaveLength(2);

    await selectButtons[0].vm.$emit("update:modelValue", "zh-CN");
    await selectButtons[1].vm.$emit("update:modelValue", "light");

    expect(i18n.global.locale.value).toBe("zh-CN");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(window.localStorage.getItem("context69.theme")).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("ignores empty reselect events for locale and theme", async () => {
    const i18n = createAppI18n("en");
    const wrapper = mount(AppFooterTools, {
      global: {
        plugins: [testPrimeVuePlugin, i18n],
      },
    });

    const selectButtons = wrapper.findAllComponents(SelectButton);

    await selectButtons[0].vm.$emit("update:modelValue", null);
    await selectButtons[1].vm.$emit("update:modelValue", null);

    expect(i18n.global.locale.value).toBe("en");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBeNull();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(window.localStorage.getItem("context69.theme")).toBe("dark");
  });
});
