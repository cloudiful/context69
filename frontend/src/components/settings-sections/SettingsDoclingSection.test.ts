import { mount } from "@vue/test-utils";
import { nextTick, reactive } from "vue";
import { describe, expect, it } from "vitest";

import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";
import { createDoclingDraft, type DraftDoclingSettings } from "../../utils/settings";
import AppSelectField from "../AppSelectField.vue";
import AppNumberField from "../AppNumberField.vue";

import SettingsDoclingSection from "./SettingsDoclingSection.vue";

function createDraft(): DraftDoclingSettings {
  return {
    ...createDoclingDraft(),
    connection: {
      base_url: "http://docling:5001",
      timeout_secs: 120,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
      max_inflight: 2,
    },
  };
}

function mountSection(
  draft: DraftDoclingSettings = createDraft(),
  doclingTesting = false,
  remote: Record<string, unknown> = {},
) {
  return mount(SettingsDoclingSection, {
    props: { doclingDraft: draft, doclingTesting, ...remote },
    global: {
      plugins: [testNuxtUiPlugin, createTestI18n("en")],
      stubs: {
        UTooltip: {
          template: "<div><slot /><div class='tooltip-stub'>{{ $attrs.text }}</div></div>",
        },
      },
    },
  });
}

describe("SettingsDoclingSection", () => {
  it("shows scoped cleanup copy and emits an identity-free action", async () => {
    const wrapper = mountSection();
    expect(wrapper.text()).toContain("does not stop active Context69 work");
    const button = wrapper.findAll("button").find(button => button.text().includes("Clean up stopped tasks"));
    expect(button).toBeDefined();
    await button!.trigger("click");
    expect(wrapper.emitted("clean-stopped")).toEqual([[]]);
    const denied = mountSection(createDraft(), false, { remoteErrorStatus: 403 });
    expect(denied.findAll("button").find(button => button.text().includes("Clean up stopped tasks"))!.attributes("disabled")).toBeDefined();
  });
  it("names every docling field after its settings-schema path", () => {
    const wrapper = mountSection();

    expect(wrapper.get("#docling-base-url").attributes("name")).toBe("docling.connection.base_url");
    expect(wrapper.findAllComponents(AppNumberField).map((field) => field.props("name"))).toEqual([
      "docling.connection.timeout_secs",
      "docling.connection.poll_interval_secs",
      "docling.connection.task_timeout_secs",
      "docling.connection.max_inflight",
    ]);

    const modeSelect = wrapper.findAllComponents(AppSelectField)[0];
    expect(modeSelect.props("inputId")).toBe("docling-vlm-mode");
    expect(modeSelect.props("name")).toBe("docling.vlm_mode");
  });

  it("renders the preset field only in preset mode", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);

    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);

    draft.vlm_mode = "preset";
    await nextTick();

    expect(wrapper.get("#docling-picture-description-preset").attributes("name")).toBe(
      "docling.vlm.picture_description_preset",
    );
    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
  });

  it("renders the legacy bundle with schema names only in custom mode", async () => {
    const draft = reactive(createDraft());
    draft.vlm_mode = "custom";
    const wrapper = mountSection(draft);

    expect(wrapper.get("#docling-openai-base-url").attributes("name")).toBe("docling.vlm.openai_base_url");
    expect(wrapper.get("#docling-api-key").attributes("name")).toBe("docling.vlm.api_key");
    expect(wrapper.get("#docling-vlm-pipeline-model").attributes("name")).toBe("docling.vlm.vlm_pipeline_model");
    expect(wrapper.get("#docling-picture-description-model").attributes("name")).toBe(
      "docling.vlm.picture_description_model",
    );
    expect(wrapper.get("#docling-code-formula-model").attributes("name")).toBe("docling.vlm.code_formula_model");
    expect(wrapper.find("#docling-picture-description-preset").exists()).toBe(false);

    draft.vlm_mode = "disabled";
    await nextTick();

    expect(wrapper.find("#docling-openai-base-url").exists()).toBe(false);
    expect(wrapper.find("#docling-api-key").exists()).toBe(false);
    expect(wrapper.find("#docling-picture-description-model").exists()).toBe(false);
  });

  it("binds the max in-flight helper text to the field", () => {
    const wrapper = mountSection();
    const describedBy = wrapper.get("#docling-max-inflight").attributes("aria-describedby");

    expect(describedBy).toBeTruthy();
    const help = wrapper.get(`#${describedBy}`);
    expect(help.attributes("data-slot")).toBe("help");
    expect(help.text()).toContain("Mac mini");
  });

  it("leaves the VLM mode tooltip describing the active mode", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);

    expect(wrapper.get(".tooltip-stub").text()).toContain("Turn Docling VLM off");

    draft.vlm_mode = "preset";
    await nextTick();
    expect(wrapper.get(".tooltip-stub").text()).toContain("picture-description preset only");

    draft.vlm_mode = "custom";
    await nextTick();
    expect(wrapper.get(".tooltip-stub").text()).toContain("OpenAI-compatible base URL");
    expect(wrapper.get('[data-testid="docling-vlm-mode-info"]').attributes("aria-label")).toBe("VLM mode details");
  });

  it("puts the test action in the connection block title row", () => {
    const wrapper = mountSection();
    const connection = wrapper.get("#settings-connection");

    // The action belongs to the connection block's title row, beside its title,
    // rather than in the block body: the title row is the first child of the
    // block element, the same place the other settings test actions sit.
    const titleRow = connection.element.firstElementChild!;
    expect(titleRow.textContent).toContain("Connection");
    expect(titleRow.textContent).toContain("Test Connection");
    expect(titleRow.querySelector('[data-testid="docling-connection-test"]')).not.toBeNull();
    expect(wrapper.get('[data-testid="docling-connection-test"]').text()).toBe("Test Connection");

    // It is the connection block's action, not the VLM block's.
    expect(wrapper.get("#settings-vlm").find('[data-testid="docling-connection-test"]').exists()).toBe(false);
  });

  it("disables the test action until the endpoint has a value", async () => {
    const draft = reactive(createDraft());
    const wrapper = mountSection(draft);

    draft.connection.base_url = "   ";
    await nextTick();
    expect(wrapper.get('[data-testid="docling-connection-test"]').attributes("disabled")).toBeDefined();

    draft.connection.base_url = "http://docling.internal:5001";
    await nextTick();
    expect(
      wrapper.get('[data-testid="docling-connection-test"]').attributes("disabled"),
    ).toBeUndefined();
  });

  it("disables the test action while a probe is already running", () => {
    const wrapper = mountSection(createDraft(), true);

    const button = wrapper.get('[data-testid="docling-connection-test"]');
    expect(button.attributes("disabled")).toBeDefined();
    expect(button.attributes("aria-busy")).toBe("true");
  });

  it("asks for a probe and never for a save", async () => {
    const wrapper = mountSection();

    await wrapper.get('[data-testid="docling-connection-test"]').trigger("click");

    // The section owns only the intent: it emits the probe request and exposes
    // no way to persist the draft, so a test cannot be mistaken for a save.
    expect(wrapper.emitted("test-docling")).toHaveLength(1);
    expect(wrapper.emitted()).not.toHaveProperty("update:doclingDraft");
  });

  it("shows the remote empty state before the first load", () => {
    const wrapper = mountSection();

    expect(wrapper.find("#settings-docling-remote-tasks").exists()).toBe(true);
    expect(wrapper.get('[data-testid="docling-remote-empty"]').text()).toContain(
      "No active remote tasks",
    );
  });

  it("renders remote identities verbatim, including unknown statuses", () => {
    const wrapper = mountSection(createDraft(), false, {
      remoteLoadedOnce: true,
      remoteItems: [
        {
          task_id: "task-1",
          task_type: "convert",
          task_status: "started",
          created_at: "2026-10-01T00:00:00Z",
        },
        {
          task_id: "task-2",
          task_type: "convert",
          task_status: "unknown_future",
          created_at: "2026-10-01T01:00:00Z",
        },
      ],
      remoteHasMore: true,
      remoteOffset: 0,
    });

    expect(wrapper.text()).toContain("task-1");
    expect(wrapper.text()).toContain("unknown_future");
    expect(wrapper.get('[data-testid="docling-remote-next"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.get('[data-testid="docling-remote-prev"]').attributes("disabled")).toBeDefined();
  });

  it("shows admin denial accurately on 403", () => {
    const wrapper = mountSection(createDraft(), false, {
      remoteLoadedOnce: false,
      remoteError: "admin access required",
      remoteErrorStatus: 403,
    });

    expect(wrapper.get('[data-testid="docling-remote-error"]').text()).toContain(
      "Admin access required",
    );
  });

  it("shows the upstream message for non-admin errors", () => {
    const wrapper = mountSection(createDraft(), false, {
      remoteError: "remote unavailable",
      remoteErrorStatus: 503,
    });

    expect(wrapper.get('[data-testid="docling-remote-error"]').text()).toContain("remote unavailable");
    expect(wrapper.get('[data-testid="docling-remote-error"]').text()).toContain("503");
  });

  it("emits refresh and pagination intents without inventing a total", async () => {
    const wrapper = mountSection(createDraft(), false, {
      remoteLoadedOnce: true,
      remoteItems: [
        {
          task_id: "task-1",
          task_type: "convert",
          task_status: "started",
          created_at: "2026-10-01T00:00:00Z",
        },
      ],
      remoteHasMore: true,
      remoteOffset: 20,
    });

    await wrapper.get('[data-testid="docling-remote-refresh"]').trigger("click");
    await wrapper.get('[data-testid="docling-remote-next"]').trigger("click");
    await wrapper.get('[data-testid="docling-remote-prev"]').trigger("click");

    expect(wrapper.emitted("refresh-remote")).toHaveLength(1);
    expect(wrapper.emitted("next-remote")).toHaveLength(1);
    expect(wrapper.emitted("prev-remote")).toHaveLength(1);
    expect(wrapper.text()).not.toContain("total");
  });

  function cleanupRow(overrides: Record<string, unknown> = {}) {
    return {
      id: "c-1",
      task_id: "t-1",
      remote_task_id: "remote-1",
      status: "unsupported",
      outcome: null,
      reason_code: "missing_submit_endpoint",
      attempt_count: 1,
      next_attempt_at: "2026-10-01T00:00:00Z",
      cleanup_deadline_at: "2026-10-02T00:00:00Z",
      ...overrides,
    };
  }

  it("localizes cleanup status, outcome, and reason without raw diagnostics", () => {
    const wrapper = mountSection(createDraft(), false, {
      cleanupItems: [
        cleanupRow(),
        cleanupRow({ id: "c-2", remote_task_id: "remote-2", status: "done", outcome: "cancelled", reason_code: null }),
        cleanupRow({ id: "c-3", remote_task_id: "remote-3", status: "awaiting_terminal", outcome: null, reason_code: "awaiting_remote_terminal" }),
      ],
    });

    const text = wrapper.text();
    expect(text).toContain("Unsupported");
    expect(text).toContain("Missing submit address");
    expect(text).toContain("Historical task has no submit address");
    expect(text).toContain("Cancelled");
    expect(text).toContain("Awaiting remote terminal");
    expect(text).not.toContain("Submit endpoint unavailable");
    expect(text).not.toContain("awaiting_terminal");
  });

  it("explains a missing submit endpoint without implying a server limitation", () => {
    const wrapper = mountSection(createDraft(), false, {
      cleanupItems: [cleanupRow({ reason_code: "missing_submit_endpoint" })],
    });

    expect(wrapper.text()).toContain("Historical task has no submit address; no remote cancel request was sent.");
    expect(wrapper.text()).not.toContain("does not support");
  });

  it("maps remote disappearance to an unconfirmed-cancellation explanation", () => {
    const wrapper = mountSection(createDraft(), false, {
      cleanupItems: [cleanupRow({ status: "done", outcome: "not_found", reason_code: "remote_not_found" })],
    });

    expect(wrapper.text()).toContain("Not found");
    expect(wrapper.text()).toContain("cancellation unconfirmed");
  });

  it("falls back to a generic cause for unknown codes and old responses", () => {
    const wrapper = mountSection(createDraft(), false, {
      cleanupItems: [
        cleanupRow({ reason_code: "future_code" }),
        cleanupRow({ id: "c-9", remote_task_id: "remote-9", reason_code: undefined }),
      ],
    });

    expect(wrapper.text()).toContain("Unknown cause");
    expect(wrapper.text()).not.toContain("future_code");
  });

  it("shows the woken count including zero with eligibility scope", () => {
    const zero = mountSection(createDraft(), false, { cleanupWoken: 0 });
    expect(zero.get('[data-testid="docling-cleanup-woken"]').text()).toContain("0");
    expect(zero.text()).toContain("Only pending, cancelling, or awaiting-terminal rows");

    const some = mountSection(createDraft(), false, { cleanupWoken: 2 });
    expect(some.get('[data-testid="docling-cleanup-woken"]').text()).toContain("2");

    const pristine = mountSection(createDraft(), false, {});
    expect(pristine.find('[data-testid="docling-cleanup-woken"]').exists()).toBe(false);
    expect(pristine.text()).toContain("Only pending, cancelling, or awaiting-terminal rows");
  });
});
