import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import LibraryToolbar from "./LibraryToolbar.vue";

function mountToolbar(props: Record<string, unknown> = {}) {
  return mount(LibraryToolbar, {
    props: {
      breadcrumbHome: { label: "Acme", onSelect: () => {} },
      breadcrumbItems: [],
      searchQuery: "",
      showSearch: false,
      ...props,
    },
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n()],
    },
  });
}

describe("LibraryToolbar", () => {
  it("always renders the breadcrumb root even without descendants", () => {
    const wrapper = mountToolbar({
      breadcrumbHome: { label: "Acme", onSelect: () => {} },
      breadcrumbItems: [],
    });

    expect(wrapper.find('[data-testid="library-go-up"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("Acme");
    wrapper.unmount();
  });

  it("disables Up at the virtual root and emits up when enabled", async () => {
    const root = mountToolbar({ canGoUp: false });
    const upRoot = root.get('[data-testid="library-go-up"]');
    expect(upRoot.attributes("disabled")).not.toBeUndefined();
    root.unmount();

    const nested = mountToolbar({ canGoUp: true });
    await nested.get('[data-testid="library-go-up"]').trigger("click");
    expect(nested.emitted("up")).toHaveLength(1);
    nested.unmount();
  });

  it("includes nested trail items after the home crumb", () => {
    const wrapper = mountToolbar({
      breadcrumbHome: { label: "Acme", onSelect: () => {} },
      breadcrumbItems: [
        { label: "docs", onSelect: () => {} },
        { label: "nested", onSelect: () => {} },
      ],
    });

    expect(wrapper.text()).toContain("Acme");
    expect(wrapper.text()).toContain("docs");
    expect(wrapper.text()).toContain("nested");
    wrapper.unmount();
  });
});
