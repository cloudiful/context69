import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { h, nextTick, type Component } from "vue";

import UFormField from "@nuxt/ui/components/FormField.vue";
import UInput from "@nuxt/ui/components/Input.vue";

import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import AppFormField from "./AppFormField.vue";

// UInput is generic; erase it so render-function props resolve cleanly.
const Input = UInput as unknown as Component;

function mountField(props: {
  inputId: string;
  label: string;
  helper?: string;
  layout?: "stacked" | "inline";
  name?: string;
}) {
  return mount(AppFormField, {
    props,
    slots: {
      default: () => h(Input, { id: props.inputId, modelValue: "value" }),
    },
    global: { plugins: [testNuxtUiPlugin] },
  });
}

describe("AppFormField", () => {
  it("keeps the input id, links the label to it, and wires the helper text", async () => {
    const wrapper = mountField({
      inputId: "runtime-embedding-model",
      label: "Embedding model",
      helper: "Pick a model served by the embedding endpoint.",
    });
    await nextTick();

    const input = wrapper.get("#runtime-embedding-model");

    const label = wrapper.get("label");
    expect(label.text()).toContain("Embedding model");
    expect(label.attributes("for")).toBe("runtime-embedding-model");

    const help = wrapper.get('[data-slot="help"]');
    expect(help.text()).toContain("Pick a model served by the embedding endpoint.");
    expect(input.attributes("aria-describedby")).toBe(help.attributes("id"));
  });

  it("omits the helper block when no helper text is provided", () => {
    const wrapper = mountField({ inputId: "runtime-qdrant-url", label: "Qdrant URL" });

    expect(wrapper.find('[data-slot="help"]').exists()).toBe(false);
    expect(wrapper.get("#runtime-qdrant-url").attributes("aria-describedby")).toBeUndefined();
  });

  it("defaults to a stacked layout and switches orientation for inline fields", () => {
    const stacked = mountField({ inputId: "search-mode", label: "Mode" });
    expect(stacked.get('[data-slot="root"]').attributes("data-orientation")).toBe("vertical");

    const inline = mountField({ inputId: "search-mode", label: "Mode", layout: "inline" });
    expect(inline.get('[data-slot="root"]').attributes("data-orientation")).toBe("horizontal");
  });

  it("names the underlying form field after the input id by default", () => {
    const wrapper = mountField({ inputId: "runtime-embedding-model", label: "Embedding model" });

    expect(wrapper.findComponent(UFormField).props("name")).toBe("runtime-embedding-model");
  });

  it("forwards an explicit field name for nested validation paths", () => {
    const wrapper = mountField({
      inputId: "runtime-embedding-model",
      name: "embedding.model",
      label: "Model",
    });

    expect(wrapper.findComponent(UFormField).props("name")).toBe("embedding.model");
  });
});
