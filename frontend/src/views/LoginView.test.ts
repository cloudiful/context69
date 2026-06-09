import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";

const { login } = vi.hoisted(() => ({
  login: vi.fn(),
}));

vi.mock("../services/auth", async () => {
  const actual = await vi.importActual<typeof import("../services/auth")>("../services/auth");
  return {
    ...actual,
    login,
  };
});

import LoginView from "./LoginView.vue";

async function mountView(path = "/login") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/login", name: "login", component: LoginView },
      { path: "/search", name: "search", component: { template: "<div>Search</div>" } },
    ],
  });

  router.push(path);
  await router.isReady();

  const wrapper = mount(LoginView, {
    global: {
      plugins: [testPrimeVuePlugin, router, createTestI18n()],
    },
  });

  return { router, wrapper };
}

describe("LoginView", () => {
  beforeEach(() => {
    login.mockReset();
    setGuest();
  });

  it("submits credentials and redirects to the requested page", async () => {
    login.mockResolvedValue({
      user_id: 1,
      login_name: "admin",
      display_name: "Administrator",
      is_admin: true,
      personal_group_key: "personal-admin",
      personal_group_role: "owner",
    });

    const { router, wrapper } = await mountView("/login?redirect=%2Fsearch");

    await wrapper.get("#login-name").setValue("admin");
    await wrapper.get("#login-password").setValue("secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(login).toHaveBeenCalledWith({
      login_name: "admin",
      password: "secret",
    });
    expect(router.currentRoute.value.fullPath).toBe("/search");
  });

  it("shows an invalid credential message when login fails", async () => {
    login.mockRejectedValue({
      name: "AuthError",
      reason: "invalid_credentials",
      message: "invalid login or password",
    });

    const { wrapper } = await mountView();

    await wrapper.get("#login-name").setValue("admin");
    await wrapper.get("#login-password").setValue("bad");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(wrapper.text()).toContain("Invalid login name or password.");
  });

  it("shows the session expired message when routed with an expired reason", async () => {
    const { wrapper } = await mountView("/login?reason=expired");

    expect(wrapper.text()).toContain("Your session has expired. Sign in again to continue.");
  });

  it("keeps the authenticated user caption visible when available", async () => {
    setAuthenticatedUser();

    const { wrapper } = await mountView();

    expect(wrapper.text()).toContain("Administrator");
  });
});
