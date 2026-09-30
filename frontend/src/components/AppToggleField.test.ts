import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import AppToggleField from "./AppToggleField.vue";
import AppToggleGroup from "./AppToggleGroup.vue";

describe("AppToggleField", () => {
  it("renders the label and helper text around a native switch", () => {
    const wrapper = mount(AppToggleField, {
      props: {
        inputId: "runtime-file-library-trusted-proxy",
        label: "Trusted proxy",
        helper: "Trust the forwarded client address.",
        testId: "runtime-file-library-trusted-proxy",
        modelValue: false,
      },
      global: { plugins: [testNuxtUiPlugin] },
    });

    expect(wrapper.text()).toContain("Trusted proxy");
    expect(wrapper.text()).toContain("Trust the forwarded client address.");
    expect(wrapper.find("#runtime-file-library-trusted-proxy").exists()).toBe(true);
    expect(wrapper.find('[data-testid="runtime-file-library-trusted-proxy"]').exists()).toBe(true);
    expect(wrapper.get("label").attributes("for")).toBe("runtime-file-library-trusted-proxy");
  });

  it("forwards the disabled state and model updates", async () => {
    const wrapper = mount(AppToggleField, {
      props: { inputId: "search-rerank-enabled", label: "Rerank", modelValue: false, disabled: true },
      global: { plugins: [testNuxtUiPlugin] },
    });

    const toggle = wrapper.get('[data-slot="base"]');
    expect(toggle.attributes("disabled")).toBeDefined();

    await wrapper.setProps({ disabled: false });
    expect(wrapper.get('[data-slot="base"]').attributes("disabled")).toBeUndefined();

    await wrapper.get('[data-slot="base"]').trigger("click");
    expect(wrapper.emitted("update:modelValue")?.at(-1)).toEqual([true]);
  });

  it("names each toggle from the group name and item key", () => {
    const wrapper = mount(AppToggleGroup, {
      props: {
        name: "file_library",
        items: [{ key: "s3_enabled", inputId: "runtime-file-library-s3-enabled", label: "S3" }],
        modelValue: { s3_enabled: false },
      },
      global: { plugins: [testNuxtUiPlugin] },
    });

    expect(wrapper.findComponent(AppToggleField).props("name")).toBe("file_library.s3_enabled");
  });
});
