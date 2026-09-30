import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import NotFoundView from "./NotFoundView.vue";

async function mountView() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/search", name: "search", component: { template: "<div>search</div>" } },
      { path: "/:pathMatch(.*)*", name: "not-found", component: NotFoundView },
    ],
  });

  await router.push("/missing");
  await router.isReady();

  const wrapper = mount(NotFoundView, {
    global: { plugins: [testNuxtUiPlugin, createTestI18n(), router] },
  });

  return { router, wrapper };
}

describe("NotFoundView", () => {
  it("renders the 404 copy inside the shared page shell", async () => {
    const { wrapper } = await mountView();

    expect(wrapper.find('[data-slot="root"]').exists()).toBe(true);
    expect(wrapper.find('[data-slot="center"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("404");
    expect(wrapper.text()).toContain("Page not found");
  });

  it("returns to search when the action is pressed", async () => {
    const { router, wrapper } = await mountView();

    await wrapper.get("button").trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("search");
  });
});