import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { testPrimeVuePlugin } from "../test-utils/primevue";
import AppRecordCard from "./AppRecordCard.vue";

describe("AppRecordCard", () => {
  it("renders record header slots and card content", () => {
    const wrapper = mount(AppRecordCard, {
      props: {
        title: "Quarterly Report",
        subtitle: "2026-04-10",
        description: "Summary text",
      },
      slots: {
        tags: '<span class="tag-slot">Tag</span>',
        meta: '<span class="meta-slot">/Reports</span>',
        actions: '<button class="action-slot">Open</button>',
        default: '<pre class="content-slot">Preview</pre>',
      },
      global: {
        plugins: [testPrimeVuePlugin],
      },
    });

    expect(wrapper.text()).toContain("Quarterly Report");
    expect(wrapper.text()).toContain("2026-04-10");
    expect(wrapper.text()).toContain("Summary text");
    expect(wrapper.get(".tag-slot").text()).toBe("Tag");
    expect(wrapper.get(".meta-slot").text()).toBe("/Reports");
    expect(wrapper.get(".action-slot").text()).toBe("Open");
    expect(wrapper.get(".content-slot").text()).toBe("Preview");
  });
});
