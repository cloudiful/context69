import { mount } from "@vue/test-utils";
import Select from "primevue/select";
import ToggleSwitch from "primevue/toggleswitch";
import { describe, expect, it } from "vitest";

import AppFormField from "./AppFormField.vue";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import AppSelectField from "./AppSelectField.vue";
import AppTextField from "./AppTextField.vue";
import AppToggleGroup from "./AppToggleGroup.vue";

describe("App field controls", () => {
  it("emits text input updates", async () => {
    const wrapper = mount(AppTextField, {
      props: {
        inputId: "name",
        label: "Name",
        modelValue: "",
      },
      global: {
        plugins: [testPrimeVuePlugin],
      },
    });

    await wrapper.get("#name").setValue("Context69");

    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["Context69"]);
  });

  it("emits select updates", async () => {
    const wrapper = mount(AppSelectField, {
      props: {
        inputId: "engine",
        label: "Engine",
        modelValue: "",
        options: [
          { label: "Auto", value: "" },
          { label: "RapidOCR", value: "rapidocr" },
        ],
      },
      global: {
        plugins: [testPrimeVuePlugin],
      },
    });

    await wrapper.findComponent(Select).vm.$emit("update:modelValue", "rapidocr");

    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["rapidocr"]);
  });

  it("renders an inline form-field layout when requested", () => {
    const wrapper = mount(AppFormField, {
      props: {
        inputId: "base-url",
        label: "Base URL",
        layout: "inline",
      },
      slots: {
        default: '<input id="base-url" class="p-inputtext p-component" />',
      },
    });

    expect(wrapper.classes()).toContain("md:grid-cols-[11rem_minmax(0,1fr)]");
    expect(wrapper.find("label").classes()).toContain("md:self-center");
  });

  it("renders a standard label linked to the input", () => {
    const wrapper = mount(AppTextField, {
      props: {
        inputId: "base-url",
        label: "Base URL",
        modelValue: "",
      },
      global: {
        plugins: [testPrimeVuePlugin],
      },
    });

    expect(wrapper.find("label[for='base-url']").text()).toBe("Base URL");
    expect(wrapper.get("#base-url").attributes("id")).toBe("base-url");
  });

  it("merges toggle updates by key", async () => {
    const wrapper = mount(AppToggleGroup, {
      props: {
        items: [
          { key: "ocr", inputId: "ocr", label: "OCR" },
          { key: "force", inputId: "force", label: "Force OCR" },
        ],
        modelValue: {
          ocr: false,
          force: true,
        },
      },
      global: {
        plugins: [testPrimeVuePlugin],
      },
    });

    await wrapper.findAllComponents(ToggleSwitch)[0].vm.$emit("update:modelValue", true);

    expect(wrapper.emitted("update:modelValue")?.[0]?.[0]).toEqual({
      ocr: true,
      force: true,
    });
  });
});
