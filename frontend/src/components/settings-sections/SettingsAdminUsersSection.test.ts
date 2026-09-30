import { DOMWrapper, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import type { AdminUserResponse } from "../../services/api";

import SettingsAdminUsersSection from "./SettingsAdminUsersSection.vue";

const adminUser: AdminUserResponse = {
  user_id: 1,
  login_name: "alice",
  display_name: "Alice",
  is_admin: true,
  disabled_at: null,
  created_at: "2026-06-01T00:00:00Z",
  updated_at: "2026-06-01T00:00:00Z",
};

function mountSection(overrides: Record<string, unknown> = {}) {
  return mount(SettingsAdminUsersSection, {
    attachTo: document.body,
    props: {
      busy: false,
      createBusy: false,
      pagination: { page: 1, page_size: 50, total: 1, total_pages: 1 },
      query: "",
      users: [adminUser],
      ...overrides,
    },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

type SectionWrapper = ReturnType<typeof mountSection>;

// UModal renders through a portal, so the dialog body lives in document.body.
async function openModal(wrapper: SectionWrapper, buttonLabel: string): Promise<DOMWrapper<HTMLElement>> {
  const button = wrapper.findAll("button").find((candidate) => candidate.text() === buttonLabel);
  expect(button).toBeDefined();
  await button!.trigger("click");

  const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
  expect(dialog).not.toBeNull();
  return new DOMWrapper(dialog!);
}

function labelText(modal: DOMWrapper<HTMLElement>) {
  return modal.findAll("label").map((label) => label.text());
}

function footerButton(modal: DOMWrapper<HTMLElement>, text: string) {
  const button = modal.findAll("button").find((candidate) => candidate.text() === text);
  expect(button).toBeDefined();
  return button!;
}

describe("SettingsAdminUsersSection", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("replaces the raw create labels with accessible form fields", async () => {
    const wrapper = mountSection();
    const modal = await openModal(wrapper, "Create User");

    expect(labelText(modal)).toEqual(["Login Name", "Display Name", "Password", "Administrator"]);

    const inputs = modal.findAll("input");
    expect(inputs).toHaveLength(3);
    const loginLabel = modal.findAll("label").find((label) => label.text() === "Login Name");
    expect(loginLabel!.attributes("for")).toBe(inputs[0].element.id);
    expect(inputs[0].element.id).toBeTruthy();
  });

  it("keeps create gated on the draft values and emits the create payload", async () => {
    const wrapper = mountSection();
    const modal = await openModal(wrapper, "Create User");
    const inputs = modal.findAll("input");

    expect(footerButton(modal, "Create User").attributes("disabled")).toBeDefined();

    await inputs[0].setValue("bob");
    await inputs[1].setValue("Bob");
    await inputs[2].setValue("s3cret");
    await modal.get("button[role='switch']").trigger("click");

    const createButton = footerButton(modal, "Create User");
    expect(createButton.attributes("disabled")).toBeUndefined();
    await createButton.trigger("click");

    expect(wrapper.emitted("create")).toEqual([[
      { login_name: "bob", display_name: "Bob", password: "s3cret", is_admin: true },
    ]]);
  });

  it("prefills the edit dialog and emits an update without the password", async () => {
    const wrapper = mountSection();
    const modal = await openModal(wrapper, "Edit");

    expect(modal.get('[data-slot="title"]').text()).toBe("Edit");
    expect(modal.get<HTMLInputElement>("input").element.value).toBe("Alice");
    expect(labelText(modal)).toEqual(["Display Name", "Administrator"]);

    await modal.get("input").setValue("Alice Cooper");
    await footerButton(modal, "Save").trigger("click");

    expect(wrapper.emitted("update")).toEqual([[
      { login_name: "alice", display_name: "Alice Cooper", is_admin: true },
    ]]);
  });

  it("resets the password through a dedicated dialog", async () => {
    const wrapper = mountSection();
    const modal = await openModal(wrapper, "Reset Password");

    await modal.get("input[type='password']").setValue("n3w-secret");
    await footerButton(modal, "Reset Password").trigger("click");

    expect(wrapper.emitted("resetPassword")).toEqual([[{ login_name: "alice", password: "n3w-secret" }]]);
  });
});
