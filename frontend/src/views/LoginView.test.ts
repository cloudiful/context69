import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import * as authService from "../services/auth/session";
import { AuthError } from "../services/auth/session";
import LoginView from "./LoginView.vue";

async function mountView(path = "/login") {
  const addToast = vi.fn();
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
      plugins: [
        testPrimeVuePlugin,
        {
          install(app) {
            app.config.globalProperties.$toast.add = addToast;
          },
        },
        router,
        createTestI18n(),
      ],
    },
  });

  return { addToast, router, wrapper };
}

describe("LoginView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    setGuest();
  });

  it("submits credentials and redirects to the requested page", async () => {
    const login = vi.spyOn(authService, "login").mockResolvedValue({
      user_id: 1,
      login_name: "admin",
      display_name: "Administrator",
      is_admin: true,
      personal_group_path: "personal-admin",
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
    vi.spyOn(authService, "login").mockRejectedValue(new AuthError("invalid login or password", 401, "invalid_credentials"));

    const { addToast, wrapper } = await mountView();

    await wrapper.get("#login-name").setValue("admin");
    await wrapper.get("#login-password").setValue("bad");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(addToast).toHaveBeenCalledWith({
      severity: "error",
      summary: "Error",
      detail: "Invalid login name or password.",
      life: 5000,
    });
  });

  it("shows field validation before submit", async () => {
    const login = vi.spyOn(authService, "login");
    const { wrapper } = await mountView();

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(login).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("Login Name is required.");
    expect(wrapper.text()).toContain("Password is required.");
  });

  it("keeps the authenticated user caption visible when available", async () => {
    setAuthenticatedUser();

    const { wrapper } = await mountView();

    expect(wrapper.text()).toContain("Administrator");
  });
});
