import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import Button from "primevue/button";
import Select from "primevue/select";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
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
        plugins: [testPrimeVuePlugin, createTestI18n("zh-CN")],
        stubs: { teleport: true },
      },
    });
    await flushPromises();

    const inputs = wrapper.findAll("input");
    await inputs[0].setValue("  disclosures  ");
    await inputs[1].setValue("  Disclosures  ");
    await wrapper.findComponent(Select).vm.$emit("update:modelValue", "public");
    await wrapper.findAllComponents(Button).at(-1)?.trigger("click");

    expect(wrapper.emitted("submit")?.[0]).toEqual([{
      key: "disclosures",
      name: "Disclosures",
      visibility: "public",
    }]);
  });
});
