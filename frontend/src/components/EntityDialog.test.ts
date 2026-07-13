import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import Button from "@nuxt/ui/components/Button.vue";
import Select from "@nuxt/ui/components/Select.vue";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import EntityDialog from "./EntityDialog.vue";

describe("EntityDialog", () => {
  it("trims fields and emits the selected visibility", async () => {
    const wrapper = mount(EntityDialog, {
      props: {
        visible: true,
        title: "Create group",
        showKey: true,
        initialVisibility: "private",
      },
      global: {
        plugins: [testNuxtUiPlugin, createTestI18n("zh-CN")],
        stubs: {
          Modal: {
            template: "<div><slot name='body' /><slot name='footer' /></div>",
          },
        },
      },
    });
    await flushPromises();

    const inputs = wrapper.findAll("input");
    await inputs[0].setValue("  disclosures  ");
    await inputs[1].setValue("  Disclosures  ");
    await wrapper.findComponent({ name: "Select" }).vm.$emit("update:modelValue", "public");
    await wrapper.vm.$nextTick();
    await wrapper.findAllComponents(Button).at(-1)?.trigger("click");

    expect(wrapper.emitted("submit")?.[0]).toEqual([{
      key: "disclosures",
      name: "Disclosures",
      visibility: "public",
    }]);
  });
});
