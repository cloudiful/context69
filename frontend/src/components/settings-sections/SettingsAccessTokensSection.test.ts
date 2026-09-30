import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import AppSelectField from "../AppSelectField.vue";
import AppToggleField from "../AppToggleField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";

import SettingsAccessTokensSection from "./SettingsAccessTokensSection.vue";

const scopeOptions = [
  { key: "search", label: "Search", helper: "Search APIs." },
  { key: "workspace", label: "Workspace", helper: "Groups APIs." },
  { key: "library", label: "Library", helper: "Library APIs." },
];

function createSection(overrides: Record<string, unknown> = {}) {
  const handlers = {
    createPersonalAccessToken: vi.fn().mockResolvedValue(undefined),
    confirmRevokePersonalAccessToken: vi.fn(),
    copyPersonalAccessToken: vi.fn().mockResolvedValue(undefined),
    dismissPersonalAccessTokenReveal: vi.fn(),
  };

  const wrapper = mount(SettingsAccessTokensSection, {
    props: {
      ...handlers,
      personalAccessTokenDraft: { name: "", scopes: ["search"], expires_in_days: 30 },
      personalAccessTokenCanCreate: false,
      personalAccessTokenExpiryOptions: [{ label: "30 days", value: 30 }],
      personalAccessTokenScopeOptions: scopeOptions,
      personalAccessTokenScopeToggleModel: { search: true, workspace: false, library: false },
      personalAccessTokens: [],
      personalAccessTokensPagination: { page: 1, page_size: 50, total: 0, total_pages: 0 },
      personalAccessTokensCreating: false,
      personalAccessTokensLoading: false,
      personalAccessTokensReveal: null,
      ...overrides,
    },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });

  return { wrapper, handlers };
}

describe("SettingsAccessTokensSection", () => {
  it("names the token draft fields after their draft paths", () => {
    const { wrapper } = createSection();

    expect(wrapper.get("#personal-access-token-name").attributes("name")).toBe("personal_access_tokens.name");
    expect(wrapper.getComponent(AppSelectField).props("name")).toBe("personal_access_tokens.expires_in_days");
    expect(wrapper.getComponent(AppToggleGroup).props("name")).toBe("personal_access_tokens.scopes");

    const toggleNames = wrapper.findAllComponents(AppToggleField).map((toggle) => toggle.props("name"));
    expect(toggleNames).toEqual([
      "personal_access_tokens.scopes.search",
      "personal_access_tokens.scopes.workspace",
      "personal_access_tokens.scopes.library",
    ]);
  });

  it("blocks creation until the token draft is valid and forwards the action", async () => {
    const { wrapper, handlers } = createSection();
    const createButton = wrapper.get('[data-testid="personal-access-token-create"]');

    expect(createButton.attributes("disabled")).toBeDefined();
    await createButton.trigger("click");
    expect(handlers.createPersonalAccessToken).not.toHaveBeenCalled();

    await wrapper.setProps({ personalAccessTokenCanCreate: true });
    await createButton.trigger("click");
    expect(handlers.createPersonalAccessToken).toHaveBeenCalledTimes(1);
  });

  it("reveals the one-time secret in a read-only labelled control", async () => {
    const { wrapper, handlers } = createSection({
      personalAccessTokensReveal: {
        access_token: "ctx_pat_secret",
        token: {
          token_id: "00000000-0000-0000-0000-000000000001",
          name: "CLI",
          display_prefix: "ctx_pat_abcd",
          scopes: ["search"],
          expires_at: "2027-12-31T00:00:00Z",
          last_used_at: null,
          revoked_at: null,
          created_at: "2026-06-01T00:00:00Z",
          updated_at: "2026-06-01T00:00:00Z",
        },
      },
    });

    const secret = wrapper.get<HTMLTextAreaElement>('[data-testid="personal-access-token-secret"]');
    expect(secret.element.value).toBe("ctx_pat_secret");
    expect(secret.element.readOnly).toBe(true);
    expect(secret.attributes("aria-label")).toBe("New Token");

    await wrapper.findAll("button").find((button) => button.text() === "Copy Token")!.trigger("click");
    expect(handlers.copyPersonalAccessToken).toHaveBeenCalledTimes(1);

    await wrapper.findAll("button").find((button) => button.text() === "Close")!.trigger("click");
    expect(handlers.dismissPersonalAccessTokenReveal).toHaveBeenCalledTimes(1);
  });

  it("keeps revoke independent of the token draft form", async () => {
    const revokedToken = {
      token_id: "00000000-0000-0000-0000-000000000002",
      name: "Old",
      display_prefix: "ctx_pat_old",
      scopes: ["search"],
      expires_at: "2027-12-31T00:00:00Z",
      last_used_at: null,
      revoked_at: "2026-07-01T00:00:00Z",
      created_at: "2026-06-01T00:00:00Z",
      updated_at: "2026-06-01T00:00:00Z",
    };
    const activeToken = { ...revokedToken, token_id: "00000000-0000-0000-0000-000000000003", revoked_at: null };
    const { wrapper, handlers } = createSection({
      personalAccessTokens: [activeToken, revokedToken],
      personalAccessTokensPagination: { page: 1, page_size: 50, total: 2, total_pages: 1 },
    });

    const revokeButtons = wrapper
      .findAll("button")
      .filter((button) => button.text() === "Revoke");

    expect(revokeButtons[0].attributes("disabled")).toBeUndefined();
    expect(revokeButtons[1].attributes("disabled")).toBeDefined();

    await revokeButtons[0].trigger("click");
    expect(handlers.confirmRevokePersonalAccessToken).toHaveBeenCalledWith(
      expect.objectContaining({ token_id: activeToken.token_id }),
    );
    expect(handlers.createPersonalAccessToken).not.toHaveBeenCalled();
  });
});
