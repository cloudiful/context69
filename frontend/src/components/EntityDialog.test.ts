import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import EntityDialog from "./EntityDialog.vue";

describe("EntityDialog", () => {
  it("localizes visibility option labels", () => {
    const wrapper = mount(EntityDialog, {
      props: {
        visible: true,
        title: "创建群组",
        initialVisibility: "private",
      },
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n("zh-CN")],
      },
    });

    const vm = wrapper.vm as unknown as {
      visibilityOptions?: Array<{ label: string; value: string }>;
      $?: { setupState?: { visibilityOptions?: Array<{ label: string; value: string }> } };
    };
    const options = vm.visibilityOptions ?? vm.$?.setupState?.visibilityOptions;

    expect(options).toEqual([
      { label: "私有", value: "private" },
      { label: "公开", value: "public" },
    ]);
  });
});
