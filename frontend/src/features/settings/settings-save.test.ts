import { describe, expect, it } from "vitest";

import type { DoclingSettingsResponse, SearchSettingsResponse } from "@/services/api";
import {
  buildDoclingPayload,
  buildSearchSettingsPayload,
  createDoclingDraft,
  createRuntimeDraft,
  createSearchDraft,
} from "@/utils/settings";

import {
  adoptDoclingCommit,
  adoptSearchCommit,
  captureSettingsSnapshot,
  defineSettingsSavePeer,
  runSettingsSave,
  settingsSaveHasPendingEdit,
  type SettingsSaveSnapshot,
} from "./settings-save";
import type { AppearanceValues } from "./use-appearance-draft";

function makeDoclingResponse(overrides: Partial<DoclingSettingsResponse> = {}): DoclingSettingsResponse {
  return {
    configured: true,
    source: "database",
    connection: {
      base_url: "http://docling:5001",
      timeout_secs: 120,
      poll_interval_secs: 2,
      task_timeout_secs: 600,
      max_inflight: 2,
    },
    ocr_preset: "rapidocr",
    do_ocr: true,
    do_picture_description: false,
    do_code_enrichment: false,
    do_formula_enrichment: false,
    vlm: {
      openai_base_url: null,
      has_api_key: false,
      vlm_pipeline_model: null,
      picture_description_model: null,
      code_formula_model: null,
      picture_description_preset: null,
    },
    ...overrides,
  };
}

function makeSearchResponse(overrides: Partial<SearchSettingsResponse> = {}): SearchSettingsResponse {
  return {
    mode: "hybrid",
    rerank_enabled: true,
    rerank_base_url: "https://openrouter.ai/api/v1",
    rerank_model: "cohere/rerank-4-fast",
    has_api_key: false,
    candidate_limit: 40,
    timeout_secs: 10,
    vector_weight: 0.55,
    keyword_weight: 0.35,
    ...overrides,
  };
}

/** A promise whose resolution the test controls. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolveFn, rejectFn) => {
    resolve = resolveFn;
    reject = rejectFn;
  });
  return { promise, resolve, reject };
}

function captureSnapshot(overrides: Partial<{
  doclingBaseUrl: string;
  rerankApiKey: string;
  doclingRevision: number;
  searchRevision: number;
  runtimeApplicable: boolean;
  doclingApplicable: boolean;
  searchApplicable: boolean;
  translationApplicable: boolean;
}> = {}): SettingsSaveSnapshot {
  const docling = createDoclingDraft();
  docling.connection.base_url = overrides.doclingBaseUrl ?? "http://docling:5001";
  const search = createSearchDraft();
  return captureSettingsSnapshot({
    runtime: createRuntimeDraft(),
    docling,
    search,
    searchRerankApiKey: overrides.rerankApiKey ?? "",
    translation: [],
    revisions: {
      runtime: 0,
      docling: overrides.doclingRevision ?? 1,
      search: overrides.searchRevision ?? 1,
      translation: 0,
    },
    applicable: {
      runtime: overrides.runtimeApplicable ?? false,
      docling: overrides.doclingApplicable ?? true,
      search: overrides.searchApplicable ?? true,
      translation: overrides.translationApplicable ?? false,
    },
  });
}

/** The appearance a save starts from. */
const appearance = {
  locale: "en",
  theme: "system",
  timeZone: "UTC",
} as const satisfies AppearanceValues;

describe("the shared settings save", () => {
  it("keeps the click-time body and the newer draft when an edit lands while the save is pending", async () => {
    const snapshot = captureSnapshot({ doclingBaseUrl: "http://docling:5001" });
    const pending = deferred<DoclingSettingsResponse>();
    const dispatched: unknown[] = [];
    let visibleDraft = createDoclingDraft();
    let currentRevision = snapshot.revisions.docling;

    const docling = defineSettingsSavePeer({
      key: "docling",
      save: () => {
        dispatched.push(buildDoclingPayload(snapshot.docling));
        return pending.promise;
      },
      adopt: (response) => {
        visibleDraft = adoptDoclingCommit(response);
      },
      isCurrent: () => currentRevision === snapshot.revisions.docling,
    });

    const running = runSettingsSave({ admission: null, peers: [docling] });

    // The operator edits the draft again while the request is in flight.
    visibleDraft.connection.base_url = "http://docling:5002";
    currentRevision += 1;
    pending.resolve(makeDoclingResponse());

    const outcome = await running;

    expect(dispatched).toEqual([
      expect.objectContaining({ connection: expect.objectContaining({ base_url: "http://docling:5001" }) }),
    ]);
    expect(outcome.failures).toEqual([]);
    expect(outcome.peers).toEqual([{ key: "docling", status: "fulfilled", adopted: false }]);
    // The newer edit survives the late response.
    expect(visibleDraft.connection.base_url).toBe("http://docling:5002");
  });

  it("adopts an unchanged commit and clears the pending credential", async () => {
    const snapshot = captureSnapshot({ rerankApiKey: "sk-pending" });
    let visibleSearch = createSearchDraft();
    let visibleKey = snapshot.searchRerankApiKey;
    const currentRevision = snapshot.revisions.search;

    const search = defineSettingsSavePeer({
      key: "search",
      save: () => Promise.resolve(makeSearchResponse({ rerank_model: "cohere/rerank-4-fast" })),
      adopt: (response) => {
        const adopted = adoptSearchCommit(response);
        visibleSearch = adopted.search;
        visibleKey = adopted.searchRerankApiKey;
      },
      isCurrent: () => currentRevision === snapshot.revisions.search,
    });

    const outcome = await runSettingsSave({ admission: null, peers: [search] });

    expect(outcome.failures).toEqual([]);
    expect(outcome.peers).toEqual([{ key: "search", status: "fulfilled", adopted: true }]);
    expect(visibleSearch.rerank_model).toBe("cohere/rerank-4-fast");
    // The submitted credential is not kept in the browser after the commit.
    expect(visibleKey).toBe("");
  });

  it("keeps the successful peer when another peer is rejected", async () => {
    const snapshot = captureSnapshot();
    const adopted: string[] = [];

    const docling = defineSettingsSavePeer({
      key: "docling",
      save: () => Promise.resolve(makeDoclingResponse()),
      adopt: () => {
        adopted.push("docling");
      },
      isCurrent: () => true,
    });
    const search = defineSettingsSavePeer({
      key: "search",
      save: () => Promise.reject(new Error("search rejected")),
      adopt: () => {
        adopted.push("search");
      },
      isCurrent: () => true,
    });

    const outcome = await runSettingsSave({ admission: null, peers: [docling, search] });

    expect(adopted).toEqual(["docling"]);
    expect(outcome.admissionFailed).toBe(false);
    expect(outcome.peers).toEqual([
      { key: "docling", status: "fulfilled", adopted: true },
      { key: "search", status: "rejected", adopted: false },
    ]);
    expect(outcome.failures).toHaveLength(1);
    expect(outcome.failures[0].key).toBe("search");
  });

  it("keeps the pending credential when the peer is rejected", async () => {
    const snapshot = captureSnapshot({ rerankApiKey: "sk-pending" });
    let visibleKey = snapshot.searchRerankApiKey;

    const search = defineSettingsSavePeer({
      key: "search",
      save: () => Promise.reject(new Error("search rejected")),
      adopt: (response) => {
        visibleKey = adoptSearchCommit(response).searchRerankApiKey;
      },
      isCurrent: () => true,
    });

    const outcome = await runSettingsSave({ admission: null, peers: [search] });

    expect(outcome.failures).toHaveLength(1);
    expect(visibleKey).toBe("sk-pending");
  });

  it("dispatches no peer when the admission step is rejected", async () => {
    const snapshot = captureSnapshot();
    let peerDispatched = false;

    const admission = defineSettingsSavePeer({
      key: "runtime",
      save: () => Promise.reject(new Error("runtime rejected")),
      adopt: () => undefined,
      isCurrent: () => true,
    });
    const search = defineSettingsSavePeer({
      key: "search",
      save: () => {
        peerDispatched = true;
        return Promise.resolve(makeSearchResponse());
      },
      adopt: () => undefined,
      isCurrent: () => true,
    });

    const outcome = await runSettingsSave({ admission, peers: [search] });

    expect(peerDispatched).toBe(false);
    expect(outcome.admissionFailed).toBe(true);
    expect(outcome.peers).toEqual([]);
    expect(outcome.failures[0].key).toBe("runtime");
  });

  it("keeps the pending credential when a newer edit lands while the save is pending", async () => {
    const snapshot = captureSnapshot({ rerankApiKey: "sk-pending" });
    const pending = deferred<SearchSettingsResponse>();
    let visibleKey = snapshot.searchRerankApiKey;
    let currentRevision = snapshot.revisions.search;

    const search = defineSettingsSavePeer({
      key: "search",
      save: () => pending.promise,
      adopt: (response) => {
        visibleKey = adoptSearchCommit(response).searchRerankApiKey;
      },
      isCurrent: () => currentRevision === snapshot.revisions.search,
    });

    const running = runSettingsSave({ admission: null, peers: [search] });
    currentRevision += 1;
    pending.resolve(makeSearchResponse());

    const outcome = await running;

    expect(outcome.peers).toEqual([{ key: "search", status: "fulfilled", adopted: false }]);
    expect(visibleKey).toBe("sk-pending");
  });

  it("adopts the committed response of an unchanged resource", async () => {
    const snapshot = captureSnapshot();
    let visibleDraft = createDoclingDraft();
    let currentRevision = snapshot.revisions.docling;

    const docling = defineSettingsSavePeer({
      key: "docling",
      save: () => Promise.resolve(makeDoclingResponse({
        connection: {
          base_url: "http://docling:5001",
          timeout_secs: 120,
          poll_interval_secs: 2,
          task_timeout_secs: 600,
          max_inflight: 2,
        },
      })),
      adopt: (response) => {
        visibleDraft = adoptDoclingCommit(response);
      },
      isCurrent: () => currentRevision === snapshot.revisions.docling,
    });

    const outcome = await runSettingsSave({ admission: null, peers: [docling] });

    expect(outcome.peers).toEqual([{ key: "docling", status: "fulfilled", adopted: true }]);
    expect(visibleDraft.connection.base_url).toBe("http://docling:5001");
    // An unchanged completion reports no pending edit, so the save may claim
    // the global success.
    expect(settingsSaveHasPendingEdit({
      baseline: snapshot.revisions,
      current: snapshot.revisions,
      appearanceBaseline: appearance,
      appearance,
    })).toBe(false);
  });

  it("keeps the page pending when an initially clean Search is edited while the runtime admission is pending", async () => {
    const snapshot = captureSnapshot({
      runtimeApplicable: true,
      doclingApplicable: false,
      searchApplicable: false,
    });
    const pending = deferred<{ committed: boolean }>();
    let searchRevision = snapshot.revisions.search;

    const runtime = defineSettingsSavePeer({
      key: "runtime",
      save: () => pending.promise,
      adopt: () => undefined,
      isCurrent: () => true,
    });

    const running = runSettingsSave({ admission: runtime, peers: [] });

    // The operator edits the Search, which was clean when Save was clicked.
    searchRevision += 1;
    pending.resolve({ committed: true });

    const outcome = await running;

    expect(outcome.admissionFailed).toBe(false);
    expect(outcome.failures).toEqual([]);
    expect(settingsSaveHasPendingEdit({
      baseline: snapshot.revisions,
      current: { ...snapshot.revisions, search: searchRevision },
      appearanceBaseline: appearance,
      appearance,
    })).toBe(true);
  });

  it("keeps the page pending when an initially clean Appearance is edited while a service is pending", async () => {
    const snapshot = captureSnapshot();
    const pending = deferred<DoclingSettingsResponse>();
    let visibleDraft = createDoclingDraft();

    const docling = defineSettingsSavePeer({
      key: "docling",
      save: () => pending.promise,
      adopt: (response) => {
        visibleDraft = adoptDoclingCommit(response);
      },
      isCurrent: () => true,
    });

    const running = runSettingsSave({ admission: null, peers: [docling] });

    // The operator edits the appearance, which was clean when Save was
    // clicked; the save must not commit it silently.
    const editedAppearance = { ...appearance, theme: "dark" as const };
    pending.resolve(makeDoclingResponse());

    const outcome = await running;

    expect(outcome.failures).toEqual([]);
    expect(settingsSaveHasPendingEdit({
      baseline: snapshot.revisions,
      current: snapshot.revisions,
      appearanceBaseline: appearance,
      appearance: editedAppearance,
    })).toBe(true);
    // The committed Docling response is still adopted.
    expect(visibleDraft.connection.base_url).toBe("http://docling:5001");
  });

  it("redacts the Docling credential on the adopted commit", async () => {
    const snapshot = captureSnapshot();
    const draft = createDoclingDraft();
    draft.vlm_mode = "custom";
    draft.vlm.api_key = "sk-secret";
    const captured = captureSettingsSnapshot({
      runtime: createRuntimeDraft(),
      docling: draft,
      search: createSearchDraft(),
      searchRerankApiKey: "",
      translation: [],
      revisions: { runtime: 0, docling: 1, search: 1, translation: 0 },
      applicable: { runtime: false, docling: true, search: false, translation: false },
    });
    let visibleDraft = draft;
    const currentRevision = captured.revisions.docling;

    const docling = defineSettingsSavePeer({
      key: "docling",
      save: () => Promise.resolve(makeDoclingResponse()),
      adopt: (response) => {
        visibleDraft = adoptDoclingCommit(response);
      },
      isCurrent: () => currentRevision === captured.revisions.docling,
    });

    await runSettingsSave({ admission: null, peers: [docling] });

    expect(visibleDraft.vlm.api_key).toBe("");
  });
  it("captures a snapshot that a later edit cannot change", () => {
    const docling = createDoclingDraft();
    docling.connection.base_url = "http://docling:5001";
    const snapshot = captureSettingsSnapshot({
      runtime: createRuntimeDraft(),
      docling,
      search: createSearchDraft(),
      searchRerankApiKey: "sk-pending",
      translation: [],
      revisions: { runtime: 0, docling: 1, search: 1, translation: 0 },
      applicable: { runtime: false, docling: true, search: true, translation: false },
    });

    docling.connection.base_url = "http://docling:5002";

    expect(buildDoclingPayload(snapshot.docling).connection.base_url).toBe("http://docling:5001");
    expect(buildSearchSettingsPayload(snapshot.search, snapshot.searchRerankApiKey, false).api_key)
      .toEqual({ op: "set", value: "sk-pending" });
  });
});
