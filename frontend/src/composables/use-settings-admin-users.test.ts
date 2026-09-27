import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type AdminUserPageResponse, type AdminUserResponse } from "../services/api";
import { setAuthenticatedUser, setGuest } from "../test-utils/auth";
import { createTestI18n } from "../test-utils/i18n";
import { useSettingsAdminUsers } from "./use-settings-admin-users";

const mocks = vi.hoisted(() => ({ addToast: vi.fn() }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.addToast }) }));

const adminUser: AdminUserResponse = {
  user_id: 2,
  login_name: "operator",
  display_name: "Operator",
  is_admin: false,
  created_at: "2026-08-01T00:00:00Z",
  updated_at: "2026-08-01T00:00:00Z",
  disabled_at: null,
};

function page(items: AdminUserResponse[], pageNumber = 1, pageSize = 50): AdminUserPageResponse {
  return {
    items,
    pagination: {
      page: pageNumber,
      page_size: pageSize,
      total: items.length,
      total_pages: items.length === 0 ? 0 : Math.ceil(items.length / pageSize),
    },
  };
}

function mountAdminUsers() {
  let state!: ReturnType<typeof useSettingsAdminUsers>;
  const wrapper = mount(defineComponent({
    setup() {
      state = useSettingsAdminUsers();
      return {};
    },
    template: "<div />",
  }), {
    global: { plugins: [createTestI18n("en")] },
  });

  return { state, wrapper };
}

describe("useSettingsAdminUsers", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.addToast.mockReset();
    setAuthenticatedUser();
  });

  it("skips the request and clears users when the session is not admin", async () => {
    const listAdminUsers = vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue(page([adminUser]));
    setGuest();

    const { state, wrapper } = mountAdminUsers();
    state.adminUsers.value = [adminUser];
    await state.loadAdminUsers();

    expect(listAdminUsers).not.toHaveBeenCalled();
    expect(state.adminUsers.value).toEqual([]);
    expect(state.adminUsersBusy.value).toBe(false);
    wrapper.unmount();
  });

  it("loads admin users with the trimmed query and adopts the server page", async () => {
    const listAdminUsers = vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue(page([adminUser], 2, 25));

    const { state, wrapper } = mountAdminUsers();
    state.adminUsersQuery.value = "  operator  ";
    await state.loadAdminUsers();

    expect(listAdminUsers).toHaveBeenCalledWith({
      page: 1,
      page_size: 50,
      query: "operator",
      sort_by: undefined,
      sort_direction: undefined,
    });
    expect(state.adminUsers.value).toEqual([adminUser]);
    expect(state.adminUsersPage.value).toEqual(page([adminUser], 2, 25));
    expect(state.adminUsersPageNumber.value).toBe(2);
    expect(state.adminUsersPageSize.value).toBe(25);
    wrapper.unmount();
  });

  it("exposes the pending load state until the response arrives", async () => {
    let resolveRequest!: (value: AdminUserPageResponse) => void;
    vi.spyOn(apiClient, "listAdminUsers").mockReturnValue(new Promise((resolve) => {
      resolveRequest = resolve;
    }));

    const { state, wrapper } = mountAdminUsers();
    const pending = state.loadAdminUsers();
    expect(state.adminUsersBusy.value).toBe(true);

    resolveRequest(page([adminUser]));
    await pending;

    expect(state.adminUsersBusy.value).toBe(false);
    expect(state.adminUsers.value).toEqual([adminUser]);
    wrapper.unmount();
  });

  it("resets to the first page when the page size or sort changes", async () => {
    const listAdminUsers = vi.spyOn(apiClient, "listAdminUsers").mockImplementation((params) =>
      Promise.resolve(page([adminUser], params.page ?? 1, params.page_size ?? 50)));

    const { state, wrapper } = mountAdminUsers();
    state.changeAdminUsersPage(3);
    await flushPromises();
    expect(listAdminUsers).toHaveBeenLastCalledWith(expect.objectContaining({ page: 3, page_size: 50 }));

    state.changeAdminUsersPageSize(25);
    await flushPromises();
    expect(listAdminUsers).toHaveBeenLastCalledWith(expect.objectContaining({ page: 1, page_size: 25 }));

    state.changeAdminUsersPageSize(25);
    await flushPromises();
    expect(listAdminUsers).toHaveBeenCalledTimes(2);

    state.changeAdminUsersSort({ field: "login_name", direction: "asc" });
    await flushPromises();
    expect(listAdminUsers).toHaveBeenLastCalledWith(expect.objectContaining({
      page: 1,
      sort_by: "login_name",
      sort_direction: "asc",
    }));

    state.changeAdminUsersSort({ field: "login_name", direction: "asc" });
    await flushPromises();
    expect(listAdminUsers).toHaveBeenCalledTimes(3);

    state.changeAdminUsersSort(null);
    await flushPromises();
    expect(listAdminUsers).toHaveBeenLastCalledWith(expect.objectContaining({
      page: 1,
      sort_by: undefined,
      sort_direction: undefined,
    }));
    wrapper.unmount();
  });

  it("creates an admin user and reloads the list", async () => {
    const createAdminUser = vi.spyOn(apiClient, "createAdminUser").mockResolvedValue(adminUser);
    const listAdminUsers = vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue(page([adminUser]));

    const { state, wrapper } = mountAdminUsers();
    const payload = {
      login_name: "operator",
      display_name: "Operator",
      password: "secret",
      is_admin: true,
    };
    await state.createAdminUser(payload);

    expect(createAdminUser).toHaveBeenCalledWith(payload);
    expect(listAdminUsers).toHaveBeenCalledTimes(1);
    expect(state.adminUsers.value).toEqual([adminUser]);
    expect(state.adminUsersCreateBusy.value).toBe(false);
    wrapper.unmount();
  });

  it("updates, resets, disables, and enables admin users through the generated api", async () => {
    const listAdminUsers = vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue(page([adminUser]));
    const updateAdminUser = vi.spyOn(apiClient, "updateAdminUser").mockResolvedValue(adminUser);
    const resetAdminUserPassword = vi.spyOn(apiClient, "resetAdminUserPassword").mockResolvedValue(adminUser);
    const disableAdminUser = vi.spyOn(apiClient, "disableAdminUser").mockResolvedValue(adminUser);
    const enableAdminUser = vi.spyOn(apiClient, "enableAdminUser").mockResolvedValue(adminUser);

    const { state, wrapper } = mountAdminUsers();
    await state.updateAdminUser({ login_name: "operator", display_name: "Ops", is_admin: true });
    expect(updateAdminUser).toHaveBeenCalledWith("operator", { display_name: "Ops", is_admin: true });

    await state.resetAdminUserPassword({ login_name: "operator", password: "new-secret" });
    expect(resetAdminUserPassword).toHaveBeenCalledWith("operator", { password: "new-secret" });

    await state.disableAdminUser("operator");
    expect(disableAdminUser).toHaveBeenCalledWith("operator");

    await state.enableAdminUser("operator");
    expect(enableAdminUser).toHaveBeenCalledWith("operator");

    expect(listAdminUsers).toHaveBeenCalledTimes(4);
    expect(state.adminUsersBusy.value).toBe(false);
    wrapper.unmount();
  });

  it("reports a failed load without dropping the current users", async () => {
    vi.spyOn(apiClient, "listAdminUsers").mockRejectedValue(new Error("boom"));

    const { state, wrapper } = mountAdminUsers();
    state.adminUsers.value = [adminUser];
    await state.loadAdminUsers();

    expect(state.adminUsers.value).toEqual([adminUser]);
    expect(state.adminUsersBusy.value).toBe(false);
    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({
      color: "error",
      description: "boom",
    }));
    wrapper.unmount();
  });

  it("resets the create busy flag when the request fails", async () => {
    vi.spyOn(apiClient, "createAdminUser").mockRejectedValue(new Error("conflict"));
    const listAdminUsers = vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue(page([]));

    const { state, wrapper } = mountAdminUsers();
    await state.createAdminUser({
      login_name: "operator",
      display_name: "Operator",
      password: "secret",
      is_admin: false,
    });

    expect(state.adminUsersCreateBusy.value).toBe(false);
    expect(listAdminUsers).not.toHaveBeenCalled();
    expect(mocks.addToast).toHaveBeenCalledWith(expect.objectContaining({ color: "error" }));
    wrapper.unmount();
  });
});
