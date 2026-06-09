import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { testPrimeVuePlugin } from "../test-utils/primevue";
import AsyncStateBlock from "./AsyncStateBlock.vue";

describe("AsyncStateBlock", () => {
  it("renders loading, error, empty, and ready states", async () => {
    const wrapper = mount(AsyncStateBlock, {
      props: {
        loading: true,
        loadingTitle: "Loading",
        loadingMessage: "Please wait",
        loadingTestId: "loading-indicator",
      },
      slots: {
        default: '<div class="ready-state">Ready</div>',
      },
      global: {
        plugins: [testPrimeVuePlugin],
      },
    });

    wrapper.get('[data-testid="loading-indicator"]');
    expect(wrapper.text()).toContain("Loading");
    expect(wrapper.text()).toContain("Please wait");

    await wrapper.setProps({
      loading: false,
      error: "Request failed",
      errorTitle: "Error",
    });

    expect(wrapper.text()).toContain("Error");
    expect(wrapper.text()).toContain("Request failed");

    await wrapper.setProps({
      error: "",
      empty: true,
      emptyTitle: "No Data",
      emptyMessage: "Nothing available",
    });

    expect(wrapper.text()).toContain("No Data");
    expect(wrapper.text()).toContain("Nothing available");

    await wrapper.setProps({
      empty: false,
    });

    expect(wrapper.get(".ready-state").text()).toBe("Ready");
  });
});
