/**
 * The settings page's save flow: the runtime section is the admission gate, so
 * it is dispatched first and the remaining sections only follow once it has
 * committed, in parallel, and a rejection anywhere reports that section's own
 * reason instead of a generic fallback.
 *
 * The connection tests live here too: both use the persisted runtime draft
 * without saving it.
 */
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  installSettingsViewSpies,
  mountSettingsView,
  runtimeResponse,
  surfacedReasons,
  type SettingsApiSpies,
} from "./settings-view-test-support";
import AppNumberField from "../components/AppNumberField.vue";
import { apiClient } from "../services/api";
import * as nuxtUiComposables from "@nuxt/ui/composables";

let apiSpies: SettingsApiSpies;

/** Qdrant probe outcomes each carry a distinct id under this prefix. */
const QDRANT_TOAST_ID_PREFIX = "runtime-qdrant-test-";

function isQdrantToast(toast: unknown) {
  const id = (toast as { id?: unknown }).id;
  return typeof id === "string" && id.startsWith(QDRANT_TOAST_ID_PREFIX);
}

describe("SettingsView save flow", () => {
  beforeEach(() => {
    apiSpies = installSettingsViewSpies();
  });

  it("sends the runtime save first and dispatches no other section when it is rejected", async () => {
    // The backend rejects an embedding identity edit while the vector index is
    // live; the page relies on that authoritative response rather than a local
    // preflight.
    const backendError = new Error(
      "runtime.embedding model/base URL/dimensions cannot change while the vector index is live",
    );
    apiSpies.updateRuntimeSettings.mockRejectedValue(backendError);

    const { wrapper, router } = await mountSettingsView("/settings/runtime");
    await vi.waitFor(() => {
      expect(wrapper.find("#runtime-embedding-model").exists()).toBe(true);
    });

    // An identity change on the runtime section (model), plus changes in other
    // sections that must not be dispatched once the runtime save is rejected.
    await wrapper.get("#runtime-embedding-model").setValue("text-embedding-3-small");
    await router.push("/settings/docling");
    await flushPromises();
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await router.push("/settings/search");
    await flushPromises();
    const vectorWeightField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "search-vector-weight");
    await vectorWeightField!.vm.$emit("update:modelValue", 0.65);

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    // The runtime request is the only save dispatched; the rejection stops the
    // remaining sections from ever being sent.
    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateDoclingSettings).not.toHaveBeenCalled();
    expect(apiSpies.updateSearchSettings).not.toHaveBeenCalled();
    expect(apiSpies.updateTranslationSettings).not.toHaveBeenCalled();

    // The backend's own reason is surfaced, not a generic fallback, and the
    // success message is never shown.
    expect(surfacedReasons()).toContain(backendError.message);
    expect(wrapper.text()).not.toContain("Settings saved");
  });

  it("surfaces the backend validation error for a cleared dimensions field", async () => {
    // A cleared number field normalizes to 0; the API answers with the accurate
    // validation message and the page surfaces it rather than inferring an
    // identity rejection locally.
    const validationError = new Error(
      "runtime.embedding.dimensions must be greater than 0",
    );
    apiSpies.updateRuntimeSettings.mockRejectedValue(validationError);

    const { wrapper } = await mountSettingsView("/settings/runtime");
    await vi.waitFor(() => {
      expect(wrapper.find("#runtime-embedding-dimensions").exists()).toBe(true);
    });

    const dimensionsField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "runtime-embedding-dimensions");
    expect(dimensionsField).toBeDefined();
    await dimensionsField!.vm.$emit("update:modelValue", null);

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateSearchSettings).not.toHaveBeenCalled();
    expect(surfacedReasons()).toContain(validationError.message);
  });

  it("holds the other sections until the runtime save succeeds", async () => {
    const { wrapper, router } = await mountSettingsView("/settings/runtime");
    await vi.waitFor(() => {
      expect(wrapper.find("#runtime-embedding-api-key").exists()).toBe(true);
    });

    // Hold the runtime save open so the dispatch order is observable.
    let releaseRuntime = () => {};
    const runtimeGate = new Promise<void>((resolve) => {
      releaseRuntime = resolve;
    });
    apiSpies.updateRuntimeSettings.mockImplementation(
      (() => runtimeGate.then(() => runtimeResponse)) as never,
    );

    await wrapper.get("#runtime-embedding-api-key").setValue("rotated-secret");
    await router.push("/settings/docling");
    await flushPromises();
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await router.push("/settings/search");
    await flushPromises();
    const vectorWeightField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "search-vector-weight");
    await vectorWeightField!.vm.$emit("update:modelValue", 0.65);

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    // While the runtime save is pending, no other section has been dispatched.
    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateDoclingSettings).not.toHaveBeenCalled();
    expect(apiSpies.updateSearchSettings).not.toHaveBeenCalled();

    releaseRuntime();
    await flushPromises();

    // Once the runtime save succeeds, the remaining sections are dispatched.
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateSearchSettings).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("Settings saved");
  });

  it("dispatches the remaining sections in parallel after a successful runtime save", async () => {
    const { wrapper, router } = await mountSettingsView("/settings/runtime");
    await vi.waitFor(() => {
      expect(wrapper.find("#runtime-embedding-api-key").exists()).toBe(true);
    });

    // The runtime save succeeds immediately; hold the other sections open so a
    // sequential dispatch could not reach the last call.
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const pending = () => gate.then(() => undefined) as never;
    apiSpies.updateDoclingSettings.mockImplementation(pending);
    apiSpies.updateSearchSettings.mockImplementation(pending);

    await wrapper.get("#runtime-embedding-api-key").setValue("rotated-secret");
    await router.push("/settings/docling");
    await flushPromises();
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await router.push("/settings/search");
    await flushPromises();
    const vectorWeightField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "search-vector-weight");
    await vectorWeightField!.vm.$emit("update:modelValue", 0.65);

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    // Both remaining sections were already in flight while neither had resolved.
    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateSearchSettings).toHaveBeenCalledTimes(1);

    release();
    await flushPromises();
    expect(wrapper.text()).toContain("Settings saved");
  });

  it("skips the runtime request and keeps other sections parallel when only they changed", async () => {
    const { wrapper, router } = await mountSettingsView("/settings/runtime");
    await vi.waitFor(() => {
      expect(apiSpies.getRuntimeSettings).toHaveBeenCalledTimes(1);
    });

    // No runtime edit at all, so the admission step must be skipped entirely
    // rather than re-sending the unchanged runtime row.
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const pending = () => gate.then(() => undefined) as never;
    apiSpies.updateDoclingSettings.mockImplementation(pending);
    apiSpies.updateSearchSettings.mockImplementation(pending);

    await router.push("/settings/docling");
    await flushPromises();
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await router.push("/settings/search");
    await flushPromises();
    const vectorWeightField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "search-vector-weight");
    await vectorWeightField!.vm.$emit("update:modelValue", 0.65);

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    // The runtime leg is skipped, and the two changed sections are already in
    // flight while neither has resolved, so the parallel block was not
    // serialized behind an admission step that has nothing to admit.
    expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateSearchSettings).toHaveBeenCalledTimes(1);

    release();
    await flushPromises();
    expect(wrapper.text()).toContain("Settings saved");
  });

  it("surfaces a rejection from a later section after the runtime save already committed", async () => {
    const { wrapper, router } = await mountSettingsView("/settings/runtime");
    await vi.waitFor(() => {
      expect(wrapper.find("#runtime-embedding-api-key").exists()).toBe(true);
    });

    // The runtime leg is the admission gate and commits first; a later section
    // failing must report its own reason and must not claim success.
    const doclingError = new Error("docling backend refused the connection");
    apiSpies.updateSearchSettings.mockRejectedValue(doclingError);

    await wrapper.get("#runtime-embedding-api-key").setValue("rotated-secret");
    await router.push("/settings/docling");
    await flushPromises();
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await router.push("/settings/search");
    await flushPromises();
    const vectorWeightField = wrapper
      .findAllComponents(AppNumberField)
      .find((component) => component.props("inputId") === "search-vector-weight");
    await vectorWeightField!.vm.$emit("update:modelValue", 0.65);

    await wrapper.get("form").trigger("submit");
    await flushPromises();

    // The runtime request was sent and the docling leg still went out, because
    // the admission step already succeeded.
    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledTimes(1);
    expect(apiSpies.updateSearchSettings).toHaveBeenCalledTimes(1);
    expect(surfacedReasons()).toContain(doclingError.message);
    expect(wrapper.text()).not.toContain("Settings saved");
  });

  it("tests the current embedding settings without saving them", async () => {
    const { wrapper } = await mountSettingsView("/settings/runtime");

    await wrapper.get('[data-testid="runtime-embedding-test"]').trigger("click");
    await flushPromises();

    expect(apiSpies.testEmbeddingConnection).toHaveBeenCalledWith({
      base_url: "https://openrouter.ai/api/v1",
      model: "text-embedding-3-large",
      dimensions: 3072,
      timeout_secs: 30,
      api_key: undefined,
    });
    expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
  });

  it("tests the unsaved Qdrant URL without saving settings or touching the collection", async () => {
    const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection").mockResolvedValue(undefined as never);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      await vi.waitFor(() => {
        expect(wrapper.find('[data-testid="runtime-qdrant-test"]').exists()).toBe(true);
      });

      // The draft URL is unsaved; editing it must not trigger a save and the
      // collection value stays out of the probe payload entirely.
      await wrapper.get("#runtime-qdrant-url").setValue(" http://qdrant:6334 ");
      await wrapper.get("#runtime-qdrant-collection").setValue("other-collection");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();

      expect(qdrantSpy).toHaveBeenCalledWith({ url: "http://qdrant:6334" });
      expect(qdrantSpy.mock.calls.at(-1)?.[0]).not.toHaveProperty("collection_name");
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
    } finally {
      qdrantSpy.mockRestore();
    }
  });

  it("does not surface a stale Qdrant failure after the draft URL changes", async () => {
    const addToast = vi.fn();
    const removeToast = vi.fn();
    const toastSpy = vi
      .spyOn(nuxtUiComposables, "useToast")
      .mockReturnValue({ add: addToast, remove: removeToast } as never);
    let rejectProbe!: (reason: Error) => void;
    const probeGate = new Promise<undefined>((_resolve, reject) => {
      rejectProbe = reject;
    });
    const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection").mockReturnValue(probeGate);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      await wrapper.get("#runtime-qdrant-url").setValue("http://old-endpoint:6334");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      expect(qdrantSpy).toHaveBeenCalledWith({ url: "http://old-endpoint:6334" });

      await wrapper.get("#runtime-qdrant-url").setValue("http://new-endpoint:6334");
      rejectProbe(new Error("the old Qdrant endpoint could not be reached"));
      await flushPromises();

      expect(wrapper.get('[data-testid="runtime-qdrant-test"]').attributes("aria-busy")).toBe("false");
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
      // The invalidated failure must not surface on any toast channel.
      expect(
        addToast.mock.calls.filter(([toast]) =>
          JSON.stringify(toast).includes("the old Qdrant endpoint could not be reached"),
        ),
      ).toHaveLength(0);
      expect(
        addToast.mock.calls.filter(([toast]) => isQdrantToast(toast)),
      ).toHaveLength(0);
    } finally {
      qdrantSpy.mockRestore();
      toastSpy.mockRestore();
    }
  });

  it("does not surface a stale Qdrant success after the draft URL changes", async () => {
    const addToast = vi.fn();
    const removeToast = vi.fn();
    const toastSpy = vi
      .spyOn(nuxtUiComposables, "useToast")
      .mockReturnValue({ add: addToast, remove: removeToast } as never);
    let resolveProbe!: () => void;
    const probeGate = new Promise<undefined>((resolve) => {
      resolveProbe = () => resolve(undefined);
    });
    const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection").mockReturnValue(probeGate);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      await wrapper.get("#runtime-qdrant-url").setValue("http://old-endpoint:6334");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      expect(qdrantSpy).toHaveBeenCalledWith({ url: "http://old-endpoint:6334" });

      await wrapper.get("#runtime-qdrant-url").setValue("http://new-endpoint:6334");
      resolveProbe();
      await flushPromises();

      expect(wrapper.get('[data-testid="runtime-qdrant-test"]').attributes("aria-busy")).toBe("false");
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
      expect(
        addToast.mock.calls.filter(([toast]) => isQdrantToast(toast)),
      ).toHaveLength(0);
    } finally {
      qdrantSpy.mockRestore();
      toastSpy.mockRestore();
    }
  });

  it("removes the displayed prior Qdrant outcome when the URL changes", async () => {
    const addToast = vi.fn();
    const removeToast = vi.fn();
    const toastSpy = vi
      .spyOn(nuxtUiComposables, "useToast")
      .mockReturnValue({ add: addToast, remove: removeToast } as never);
    const qdrantSpy = vi
      .spyOn(apiClient, "testQdrantConnection")
      .mockResolvedValue(undefined as never);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      await wrapper.get("#runtime-qdrant-url").setValue("http://qdrant:6334");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();

      expect(
        addToast.mock.calls.filter(([toast]) => isQdrantToast(toast)),
      ).toHaveLength(1);
      const emittedId = (
        addToast.mock.calls.map(([toast]) => toast).find((toast) => isQdrantToast(toast)) as {
          id: string;
        }
      ).id;

      await wrapper.get("#runtime-qdrant-url").setValue("http://other-qdrant:6334");
      await flushPromises();

      expect(removeToast).toHaveBeenCalledWith(emittedId);
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
    } finally {
      qdrantSpy.mockRestore();
      toastSpy.mockRestore();
    }
  });

  it("still invalidates the old outcome when the URL is edited away and back", async () => {
    const addToast = vi.fn();
    const removeToast = vi.fn();
    const toastSpy = vi
      .spyOn(nuxtUiComposables, "useToast")
      .mockReturnValue({ add: addToast, remove: removeToast } as never);
    let rejectProbe!: (reason: Error) => void;
    const probeGate = new Promise<undefined>((_resolve, reject) => {
      rejectProbe = reject;
    });
    const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection").mockReturnValue(probeGate);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      await wrapper.get("#runtime-qdrant-url").setValue("http://old-endpoint:6334");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      expect(qdrantSpy).toHaveBeenCalledWith({ url: "http://old-endpoint:6334" });

      // Editing away and back restores the same value but bumps the generation
      // twice, so the old outcome stays invalidated.
      await wrapper.get("#runtime-qdrant-url").setValue("http://other-endpoint:6334");
      await wrapper.get("#runtime-qdrant-url").setValue("http://old-endpoint:6334");
      rejectProbe(new Error("the old Qdrant endpoint could not be reached"));
      await flushPromises();

      expect(wrapper.get('[data-testid="runtime-qdrant-test"]').attributes("aria-busy")).toBe("false");
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
      expect(
        addToast.mock.calls.filter(([toast]) => isQdrantToast(toast)),
      ).toHaveLength(0);
    } finally {
      qdrantSpy.mockRestore();
      toastSpy.mockRestore();
    }
  });

  it("keeps a fresh Qdrant result when an earlier URL edit has queued toast removal", async () => {
    const useRealToast = nuxtUiComposables.useToast;
    let realToast!: ReturnType<typeof useRealToast>;
    const toastSpy = vi.spyOn(nuxtUiComposables, "useToast").mockImplementation(() => {
      realToast = useRealToast();
      return realToast;
    });
    const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection").mockResolvedValue(undefined);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      realToast.clear();
      await wrapper.get("#runtime-qdrant-url").setValue("http://fresh-endpoint:6334");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      expect(qdrantSpy).toHaveBeenCalledWith({ url: "http://fresh-endpoint:6334" });
      expect(realToast.toasts.value.some((toast) =>
        toast.title === "Qdrant connection succeeded" && toast.open && toast.color === "success",
      )).toBe(true);

      // Nuxt UI remove closes immediately and deletes by id after 200ms. The
      // queued removal of an older outcome must not erase the newly valid one.
      await new Promise((resolve) => setTimeout(resolve, 250));
      expect(realToast.toasts.value.some((toast) =>
        toast.title === "Qdrant connection succeeded" && toast.open && toast.color === "success",
      )).toBe(true);
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
    } finally {
      realToast?.clear();
      qdrantSpy.mockRestore();
      toastSpy.mockRestore();
    }
  });

  it("keeps a fresh Qdrant error visible after a fast probe of an edited URL", async () => {
    const useRealToast = nuxtUiComposables.useToast;
    let realToast!: ReturnType<typeof useRealToast>;
    const toastSpy = vi.spyOn(nuxtUiComposables, "useToast").mockImplementation(() => {
      realToast = useRealToast();
      return realToast;
    });
    const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection")
      .mockRejectedValue(new Error("Qdrant endpoint unreachable"));
    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      realToast.clear();
      await wrapper.get("#runtime-qdrant-url").setValue("http://fresh-endpoint:6334");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      const hasError = () => realToast.toasts.value.some((toast) =>
        toast.description === "Qdrant endpoint unreachable" && toast.open && toast.color === "error",
      );
      expect(hasError()).toBe(true);
      await new Promise((resolve) => setTimeout(resolve, 250));
      expect(hasError()).toBe(true);
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
    } finally {
      realToast?.clear();
      qdrantSpy.mockRestore();
      toastSpy.mockRestore();
    }
  });

  it.each(["success", "error"] as const)(
    "clears every displayed prior Qdrant %s after repeated completed probes and a URL edit",
    async (outcome) => {
      const useRealToast = nuxtUiComposables.useToast;
      let realToast!: ReturnType<typeof useRealToast>;
      const toastSpy = vi.spyOn(nuxtUiComposables, "useToast").mockImplementation(() => {
        realToast = useRealToast();
        return realToast;
      });
      const qdrantSpy = vi.spyOn(apiClient, "testQdrantConnection");
      if (outcome === "success") {
        qdrantSpy.mockResolvedValue(undefined);
      } else {
        qdrantSpy.mockRejectedValue(new Error("old endpoint unreachable"));
      }
      try {
        const { wrapper } = await mountSettingsView("/settings/runtime");
        realToast.clear();
        await wrapper.get("#runtime-qdrant-url").setValue("http://old-endpoint:6334");
        const openQdrantOutcomes = () => realToast.toasts.value.filter((toast) =>
          toast.open && (toast.title === "Qdrant connection succeeded"
            || toast.description === "old endpoint unreachable"),
        );
        await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
        await flushPromises();
        expect(openQdrantOutcomes().length).toBeGreaterThan(0);
        expect(wrapper.get('[data-testid="runtime-qdrant-test"]').attributes("aria-busy")).toBe("false");
        await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
        await flushPromises();
        expect(qdrantSpy).toHaveBeenCalledTimes(2);
        expect(openQdrantOutcomes().length).toBeGreaterThan(0);

        await wrapper.get("#runtime-qdrant-url").setValue("http://new-endpoint:6334");
        await flushPromises();
        expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
        expect(openQdrantOutcomes()).toHaveLength(0);
      } finally {
        realToast?.clear();
        qdrantSpy.mockRestore();
        toastSpy.mockRestore();
      }
    },
  );

  it("prevents a duplicate Qdrant probe while one is in flight", async () => {
    let releaseProbe!: () => void;
    const probeGate = new Promise<void>((resolve) => {
      releaseProbe = resolve;
    });
    const qdrantSpy = vi
      .spyOn(apiClient, "testQdrantConnection")
      .mockImplementation((() => probeGate.then(() => undefined)) as never);

    try {
      const { wrapper } = await mountSettingsView("/settings/runtime");
      await vi.waitFor(() => {
        expect(wrapper.find('[data-testid="runtime-qdrant-test"]').exists()).toBe(true);
      });

      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      // While the first probe is pending the button reports busy and is
      // disabled, so a second click cannot start a duplicate request.
      expect(wrapper.get('[data-testid="runtime-qdrant-test"]').attributes("aria-busy")).toBe("true");
      await wrapper.get('[data-testid="runtime-qdrant-test"]').trigger("click");
      await flushPromises();
      expect(qdrantSpy).toHaveBeenCalledTimes(1);

      releaseProbe();
      await flushPromises();
      expect(wrapper.get('[data-testid="runtime-qdrant-test"]').attributes("aria-busy")).toBe("false");
      expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
    } finally {
      qdrantSpy.mockRestore();
    }
  });
});