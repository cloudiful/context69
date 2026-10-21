import { mount } from "@vue/test-utils";
import USelectMenu from "@nuxt/ui/components/SelectMenu.vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { installMockStorage } from "../../test-utils/storage";
import {
  AUTOMATIC_TIME_ZONE,
  TIME_ZONE_STORAGE_KEY,
  browserTimeZone,
  setTimeZonePreference,
} from "../../utils/time-zone";
import AppSelectField from "../AppSelectField.vue";

import SettingsAppearanceSection from "./SettingsAppearanceSection.vue";

function mountSection() {
  return mount(SettingsAppearanceSection, {
    props: { locale: "en", theme: "dark", timeZone: AUTOMATIC_TIME_ZONE },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

describe("SettingsAppearanceSection", () => {
  beforeEach(() => {
    installMockStorage();
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
  });

  afterEach(() => {
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
  });

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

  it("offers automatic browser detection beside explicit zones", () => {
    const wrapper = mountSection();
    const menu = wrapper.getComponent(USelectMenu);
    const menuProps = menu.props() as unknown as {
      items: Array<{ value: string; label: string }>;
      modelValue?: string;
    };
    const items = menuProps.items;

    expect(menuProps.modelValue).toBe(AUTOMATIC_TIME_ZONE);
    expect(items[0]).toEqual({
      value: AUTOMATIC_TIME_ZONE,
      label: `Automatic (${browserTimeZone})`,
    });
    expect(items.map((item) => item.value)).toContain("UTC");
    expect(items.map((item) => item.value)).toContain("Asia/Shanghai");
    expect(items.filter((item) => item.value === AUTOMATIC_TIME_ZONE)).toHaveLength(1);
  });

  it("emits a valid time zone and ignores invalid input", () => {
    const wrapper = mountSection();
    const menu = wrapper.getComponent(USelectMenu);

    menu.vm.$emit("update:modelValue", "Asia/Shanghai");
    menu.vm.$emit("update:modelValue", "+08:00");
    menu.vm.$emit("update:modelValue", AUTOMATIC_TIME_ZONE);

    expect(wrapper.emitted("update:timeZone")).toEqual([["Asia/Shanghai"], [AUTOMATIC_TIME_ZONE]]);
  });

  it("persists a selected zone through the shared preference setter", async () => {
    const wrapper = mountSection();
    const menu = wrapper.getComponent(USelectMenu);

    menu.vm.$emit("update:modelValue", "Asia/Shanghai");
    setTimeZonePreference("Asia/Shanghai");
    await wrapper.setProps({ timeZone: "Asia/Shanghai" });

    expect((wrapper.getComponent(USelectMenu).props() as { modelValue?: string }).modelValue).toBe("Asia/Shanghai");
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBe("Asia/Shanghai");
  });
});
