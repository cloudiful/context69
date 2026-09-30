import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import AppSelectField from "../AppSelectField.vue";

import SettingsAppearanceSection from "./SettingsAppearanceSection.vue";

function mountSection() {
  return mount(SettingsAppearanceSection, {
    props: { locale: "en", theme: "dark" },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

describe("SettingsAppearanceSection", () => {
  it("names the locale and theme selects after the appearance settings", () => {
    const wrapper = mountSection();
    const selects = wrapper.findAllComponents(AppSelectField);

    expect(selects.map((select) => select.props("inputId"))).toEqual([
      "settings-locale-select",
      "settings-theme-select",
    ]);
    expect(selects.map((select) => select.props("name"))).toEqual([
      "appearance.locale",
      "appearance.theme",
    ]);
    expect(selects[0].props("modelValue")).toBe("en");
    expect(selects[1].props("modelValue")).toBe("dark");
  });

  it("emits only supported locale and theme values", () => {
    const wrapper = mountSection();
    const selects = wrapper.findAllComponents(AppSelectField);

    selects[0].vm.$emit("update:modelValue", "zh-CN");
    selects[1].vm.$emit("update:modelValue", "light");
    selects[0].vm.$emit("update:modelValue", "fr");
    selects[1].vm.$emit("update:modelValue", "system");

    expect(wrapper.emitted("update:locale")).toEqual([["zh-CN"]]);
    expect(wrapper.emitted("update:theme")).toEqual([["light"]]);
  });
});
