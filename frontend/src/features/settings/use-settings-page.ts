import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { useAuthSession } from "@/hooks/use-auth";
import { apiClient } from "@/services/api";
import type {
  DoclingSettingsResponse,
  RuntimeSettingsResponse,
  SearchSettingsResponse,
  TokenizerPresetDescriptor,
  TranslationSettingsResponse,
  UpdateRuntimeSettingsRequest,
} from "@/services/api";
import {
  buildDoclingPayload,
  buildRuntimePayload,
  buildSearchSettingsComparablePayload,
  buildSearchSettingsPayload,
  createRuntimeDraft,
  doclingResponseToPayload,
  runtimeResponseToDraft,
  runtimeResponseToPayload,
  searchResponseToPayload,
  type DraftRuntimeSettings,
} from "@/utils/settings";

import { errorMessage } from "./settings-error";
import { useSettingsForm, useSettingsFormValues, type SettingsSubmitMeta } from "./settings-form";
import {
  adoptDoclingCommit,
  adoptSearchCommit,
  captureSettingsSnapshot,
  defineSettingsSavePeer,
  runSettingsSave,
  settingsSaveHasPendingEdit,
  type SettingsSavePeer,
  type TranslationProviderDraft,
} from "./settings-save";
import { useAppearanceDraft } from "./use-appearance-draft";
import { lastObservedRebuildStatus, rebuildHoldsBarrier } from "./use-vector-rebuild";

export type { TranslationProviderDraft } from "./settings-save";

export const SETTINGS_QUERY_KEYS = {
  runtime: ["settings", "runtime"] as const,
  docling: ["settings", "docling"] as const,
  search: ["settings", "search"] as const,
  translation: ["settings", "translation"] as const,
};

/**
 * The state of one tokenizer preset preparation.
 *
 * `pending` covers the retrieval of the pinned file, `ready` means this
 * process holds a verified engine for it, and `error` keeps the actionable
 * message of the last failure so the operator can retry.
 */
export type TokenizerPreparation = {
  status: "pending" | "ready" | "error";
  message?: string;
};

function cloneDraft<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/** The candidate vector-space identity a compatibility confirmation is bound to. */
export interface EmbeddingCompatibilityCandidate {
  base_url: string;
  model: string;
  dimensions: number;
}

function normalizeEndpoint(value: string) {
  return value.trim().replace(/\/+$/, "");
}

function embeddingSignature(identity: {
  base_url: string;
  model: string;
  dimensions: number;
}) {
  return JSON.stringify([
    normalizeEndpoint(identity.base_url),
    identity.model.trim(),
    identity.dimensions,
  ]);
}

/**
 * The full effective request identity of one embedding probe.
 *
 * Every input that can change what the provider checks is part of it: the
 * endpoint, the model, the distinct query alias, the dimension, the timeout,
 * the credential, the credential intent, and the unsaved document draft a
 * sparse channel request inherits from. A response is presented only while
 * this exact request is still the observed draft.
 */
function embeddingProbeSignature(request: {
  base_url?: string;
  model?: string;
  query_model?: string;
  dimensions?: number | null;
  timeout_secs?: number | null;
  api_key?: string;
  key_intent?: string;
  document_api_key?: { op: string; value?: string };
  document_base_url?: string;
  document_model?: string;
  document_query_model?: string;
  document_dimensions?: number | null;
  document_timeout_secs?: number | null;
}) {
  return JSON.stringify([
    normalizeEndpoint(request.base_url ?? ""),
    (request.model ?? "").trim(),
    (request.query_model ?? "").trim(),
    request.dimensions ?? null,
    request.timeout_secs ?? null,
    (request.api_key ?? "").trim(),
    request.key_intent ?? null,
    (request.document_api_key?.value ?? "").trim(),
    normalizeEndpoint(request.document_base_url ?? ""),
    (request.document_model ?? "").trim(),
    (request.document_query_model ?? "").trim(),
    request.document_dimensions ?? null,
    request.document_timeout_secs ?? null,
  ]);
}

/**
 * The document credential intent of a channel probe.
 *
 * The stored document secret is never hydrated into the browser. Only a key the
 * operator entered in the unsaved document draft travels with the request, and
 * it is exactly the credential the same save would install.
 */
function documentKeyPatch(document: DraftRuntimeSettings["embedding"]) {
  const value = document.api_key.trim();
  return value ? { op: "set" as const, value } : { op: "keep" as const };
}

/**
 * The probe request of the document block.
 *
 * The document configuration is complete on its own, so the request carries
 * its own values and inherits nothing. The probe must exercise the same
 * distinct query alias the deployment would use: an invalid query model
 * otherwise passes Test Connection while real query embeddings fail.
 */
function buildDocumentProbeRequest(draft: DraftRuntimeSettings["embedding"]) {
  return {
    base_url: draft.base_url.trim() || undefined,
    model: draft.model.trim() || undefined,
    query_model: draft.query_model.trim() || undefined,
    dimensions: draft.dimensions,
    timeout_secs: draft.timeout_secs,
    api_key: draft.api_key.trim() || undefined,
  };
}

/**
 * The probe request of one optional channel block.
 *
 * The request carries the same sparse overrides a save would store: a blank
 * field is absent and inherits the document configuration, and the unsaved
 * document draft travels with it as the inheritance source. The credential
 * intent is explicit: a pending clear removes the own-purpose credential, so
 * the probe then tests the inherited document credential that would be in
 * effect after the clear.
 */
function buildChannelProbeRequest(
  purpose: "code" | "image",
  draft: DraftRuntimeSettings["code_embedding"] | DraftRuntimeSettings["image_embedding"],
  document: DraftRuntimeSettings["embedding"],
) {
  return {
    purpose,
    base_url: draft.base_url.trim() || undefined,
    model: draft.model.trim() || undefined,
    query_model: draft.query_model.trim() || undefined,
    // An absent number inherits; an explicit one is a value.
    dimensions: draft.dimensions ?? undefined,
    timeout_secs: draft.timeout_secs ?? undefined,
    api_key: draft.api_key.trim() || undefined,
    key_intent: draft.clear_api_key ? "inherited" as const : "stored" as const,
    // The probe inherits from the unsaved document draft; the draft key is the
    // only document credential that travels, never the stored secret.
    document_api_key: documentKeyPatch(document),
    document_base_url: document.base_url.trim() || undefined,
    document_model: document.model.trim() || undefined,
    document_query_model: document.query_model.trim() || undefined,
    document_dimensions: document.dimensions,
    document_timeout_secs: document.timeout_secs,
  };
}

/**
 * Whether this save changes the embedding gateway/alias while keeping the
 * dimension. Only that case may keep existing vectors, and only after the
 * operator explicitly confirms both point at the same vector space. A dimension
 * change is never confirmable, so no dialog is offered for it.
 */
function needsCompatibilityConfirmation(
  candidate: { base_url: string; model: string; dimensions: number },
  saved: { base_url: string; model: string; dimensions: number },
) {
  if (candidate.dimensions !== saved.dimensions) {
    return false;
  }
  return normalizeEndpoint(candidate.base_url) !== normalizeEndpoint(saved.base_url)
    || candidate.model.trim() !== saved.model.trim();
}

/**
 * Marks the request as an explicit same-space assertion. The field is transient:
 * the backend acknowledges the persisted identity but never stores it.
 */
function withVectorSpaceConfirmation(
  payload: UpdateRuntimeSettingsRequest,
  confirm: boolean,
): UpdateRuntimeSettingsRequest {
  if (!confirm) {
    return payload;
  }
  return {
    ...payload,
    embedding: { ...payload.embedding, confirm_vector_space_unchanged: true },
  };
}

function translationPayload(providers: readonly TranslationProviderDraft[]) {
  return {
    providers: providers.map(({ has_api_key: _hasKey, current_month_characters: _usage, ...provider }) => ({
      ...provider,
      api_key: provider.api_key?.trim() || undefined,
    })),
  };
}

/**
 * Owns the settings server state, the local drafts, and the single manual Save.
 *
 * Every group keeps its own draft and a synchronous revision counter. A save
 * captures payloads and revisions before awaiting, sends runtime first as the
 * admission step, adopts each successful peer independently, and only commits
 * the captured appearance when every service save succeeded. A newer edit
 * keeps its own value and keeps the page pending.
 */
export function useSettingsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { user } = useAuthSession();
  const isAdmin = user?.is_admin === true;
  const appearanceDraft = useAppearanceDraft();

  const runtimeQuery = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.runtime,
    queryFn: () => apiClient.getRuntimeSettings(),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const doclingQuery = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.docling,
    queryFn: () => apiClient.getDoclingSettings(),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const searchQuery = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.search,
    queryFn: () => apiClient.getSearchSettings(),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const translationQuery = useQuery({
    queryKey: SETTINGS_QUERY_KEYS.translation,
    queryFn: () => apiClient.getTranslationSettings(),
    enabled: isAdmin,
    staleTime: Number.POSITIVE_INFINITY,
  });

  const [runtimeDraft, setRuntimeDraft] = useState<DraftRuntimeSettings>(createRuntimeDraft);
  const [translationProviders, setTranslationProviders] = useState<TranslationProviderDraft[]>([]);

  const runtimeRevision = useRef(0);
  const doclingRevision = useRef(0);
  const searchRevision = useRef(0);
  const translationRevision = useRef(0);
  const runtimeSeeded = useRef(0);
  const doclingSeeded = useRef(0);
  const searchSeeded = useRef(0);
  const translationSeeded = useRef(0);
  const [draftsSeeded, setDraftsSeeded] = useState(false);

  // The native form owns the Docling and the Search drafts. The provider
  // keeps its lifetime, so the drafts of every routed section survive a
  // section change, and the shared Save has one native submit entry.
  const submitHandlerRef = useRef<(meta: SettingsSubmitMeta) => void | Promise<void>>(
    () => undefined,
  );
  const form = useSettingsForm({
    onSubmit: (meta) => submitHandlerRef.current(meta),
    onResourceChange: (resource) => {
      if (resource === "docling") {
        doclingRevision.current += 1;
        return;
      }
      searchRevision.current += 1;
    },
  });
  const formValues = useSettingsFormValues(form);
  const doclingDraft = formValues.doclingSettings;
  const searchDraft = formValues.searchSettings;
  const rerankApiKeyDraft = formValues.searchRerankApiKey;

  const [saving, setSaving] = useState(false);
  // The synchronous guard against a second dispatch: an explicit Save never
  // queues another one while a save is running.
  const dispatchingRef = useRef(false);
  const [saveMessage, setSaveMessage] = useState("");

  // The preparation of the known tokenizer presets, one entry per preset.
  const [tokenizerPreparations, setTokenizerPreparations] = useState<
    Record<string, TokenizerPreparation>
  >({});
  // The preparations that are running right now, so a repeated click sends
  // one request instead of two.
  const tokenizerPreparationsInFlight = useRef(new Map<string, Promise<void>>());

  // A gateway/model change keeps existing vectors only after an explicit
  // confirmation bound to the candidate identity shown in the dialog.
  const [compatibilityOpen, setCompatibilityOpen] = useState(false);
  const [compatibilityCandidate, setCompatibilityCandidate] =
    useState<EmbeddingCompatibilityCandidate | null>(null);

  const [s3Testing, setS3Testing] = useState(false);
  const [valkeyTesting, setValkeyTesting] = useState(false);
  const [embeddingTesting, setEmbeddingTesting] = useState(false);
  const [codeEmbeddingTesting, setCodeEmbeddingTesting] = useState(false);
  const [imageEmbeddingTesting, setImageEmbeddingTesting] = useState(false);
  const [qdrantTesting, setQdrantTesting] = useState(false);
  const [doclingTesting, setDoclingTesting] = useState(false);
  const qdrantUrlRevision = useRef(0);
  const qdrantToastId = useRef<string | null>(null);
  const qdrantProbeSeq = useRef(0);

  // The full draft embedding request the operator last asked the provider
  // about. A capability or connection outcome that arrives after any probe
  // input moved (including the query alias) is discarded instead of
  // describing or validating a request the form no longer shows.
  const embeddingProbeRequest = useMemo(
    () => buildDocumentProbeRequest(runtimeDraft.embedding),
    [runtimeDraft.embedding],
  );
  const embeddingProbeTarget = embeddingProbeSignature(embeddingProbeRequest);
  const embeddingProbeTargetRef = useRef(embeddingProbeTarget);
  useEffect(() => {
    embeddingProbeTargetRef.current = embeddingProbeTarget;
  }, [embeddingProbeTarget]);

  const codeEmbeddingProbeRequest = useMemo(
    () => buildChannelProbeRequest(
      "code",
      runtimeDraft.code_embedding,
      runtimeDraft.embedding,
    ),
    [runtimeDraft.code_embedding, runtimeDraft.embedding],
  );
  const codeEmbeddingProbeTarget = embeddingProbeSignature(codeEmbeddingProbeRequest);
  const codeEmbeddingProbeTargetRef = useRef(codeEmbeddingProbeTarget);
  useEffect(() => {
    codeEmbeddingProbeTargetRef.current = codeEmbeddingProbeTarget;
  }, [codeEmbeddingProbeTarget]);

  const imageEmbeddingProbeRequest = useMemo(
    () => buildChannelProbeRequest(
      "image",
      runtimeDraft.image_embedding,
      runtimeDraft.embedding,
    ),
    [runtimeDraft.image_embedding, runtimeDraft.embedding],
  );
  const imageEmbeddingProbeTarget = embeddingProbeSignature(imageEmbeddingProbeRequest);
  const imageEmbeddingProbeTargetRef = useRef(imageEmbeddingProbeTarget);
  useEffect(() => {
    imageEmbeddingProbeTargetRef.current = imageEmbeddingProbeTarget;
  }, [imageEmbeddingProbeTarget]);

  const showErrorToast = useCallback((error: unknown, fallback: string) => {
    toast.error(t("common.error"), {
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  }, [t]);

  /**
   * Seed a group's draft from loaded data only while it carries no unsaved
   * edit, so a background refetch can never overwrite a form being edited.
   */
  const seedRuntime = useCallback((response: RuntimeSettingsResponse) => {
    if (runtimeRevision.current !== runtimeSeeded.current) {
      return;
    }
    setRuntimeDraft(runtimeResponseToDraft(response));
    runtimeSeeded.current = runtimeRevision.current;
  }, []);

  const seedDocling = useCallback((response: DoclingSettingsResponse) => {
    if (doclingRevision.current !== doclingSeeded.current) {
      return;
    }
    // A programmatic reset is not an operator edit, so it must not move the
    // resource revision.
    form.setFieldValue("doclingSettings", adoptDoclingCommit(response), { dontRunListeners: true });
    doclingSeeded.current = doclingRevision.current;
  }, [form]);

  const seedSearch = useCallback((response: SearchSettingsResponse) => {
    if (searchRevision.current !== searchSeeded.current) {
      return;
    }
    form.setFieldValue("searchSettings", adoptSearchCommit(response).search, { dontRunListeners: true });
    searchSeeded.current = searchRevision.current;
  }, [form]);

  const seedTranslation = useCallback((response: TranslationSettingsResponse) => {
    if (translationRevision.current !== translationSeeded.current) {
      return;
    }
    setTranslationProviders(response.providers.map((provider) => ({ ...provider, api_key: undefined })));
    translationSeeded.current = translationRevision.current;
  }, []);

  const runtimeData = runtimeQuery.data;
  // Persisted vectors of a different identity need an explicit rebuild; the
  // page reports it and never triggers one on its own.
  const requiresRebuild = runtimeData?.requires_rebuild === true;
  const doclingData = doclingQuery.data;
  const searchData = searchQuery.data;
  const translationData = translationQuery.data;

  /**
   * Seed every draft from the first complete load only. Later cache updates
   * never reassign a draft, so a background refetch cannot overwrite a form the
   * user is editing.
   */
  useEffect(() => {
    if (draftsSeeded) {
      return;
    }
    if (!runtimeData || !doclingData || !searchData || (isAdmin && !translationData)) {
      return;
    }
    seedRuntime(runtimeData);
    seedDocling(doclingData);
    seedSearch(searchData);
    if (translationData) {
      seedTranslation(translationData);
    }
    setDraftsSeeded(true);
  }, [
    doclingData,
    draftsSeeded,
    isAdmin,
    runtimeData,
    searchData,
    seedDocling,
    seedRuntime,
    seedSearch,
    seedTranslation,
    translationData,
  ]);

  /** Applies an edit to a cloned draft and bumps its revision synchronously. */
  const updateRuntimeDraft = useCallback((mutate: (draft: DraftRuntimeSettings) => void) => {
    runtimeRevision.current += 1;
    // Any further edit invalidates the candidate a pending confirmation named.
    setCompatibilityOpen(false);
    setCompatibilityCandidate(null);
    setRuntimeDraft((current) => {
      const next = cloneDraft(current);
      mutate(next);
      return next;
    });
  }, []);

  /**
   * Applies one code edit and marks the block pending for the next save.
   *
   * The two secret intents are mutually exclusive, so the patch itself decides
   * and not one control: an explicit clear always wins and empties the draft
   * key, even when the same patch carries a key, a nonblank replacement key
   * cancels a pending clear, and a blank key edit leaves a pending clear
   * alone. Turning the clear off never restores a discarded key.
   */
  const updateCodeEmbeddingDraft = useCallback((
    patch: Partial<DraftRuntimeSettings["code_embedding"]>,
  ) => {
    runtimeRevision.current += 1;
    setCompatibilityOpen(false);
    setCompatibilityCandidate(null);
    setRuntimeDraft((current) => {
      const next = cloneDraft(current);
      const code = next.code_embedding;
      const { clear_api_key: clearPatch, api_key: keyPatch, ...rest } = patch;
      Object.assign(code, rest);
      if (clearPatch === true) {
        // An explicit clear wins over any key the same patch carries.
        code.clear_api_key = true;
        code.api_key = "";
      } else {
        if (clearPatch === false) {
          // Turning the clear off never restores the discarded key.
          code.clear_api_key = false;
        }
        if (keyPatch !== undefined) {
          code.api_key = keyPatch;
          if (keyPatch.trim() !== "") {
            // Only a nonblank replacement key cancels a pending clear.
            code.clear_api_key = false;
          }
        }
      }
      code.touched = true;
      return next;
    });
  }, []);

  /**
   * Applies one image edit and marks the block pending for the next save.
   *
   * The two secret intents are mutually exclusive, so the patch itself decides
   * and not one control: an explicit clear always wins and empties the draft
   * key, even when the same patch carries a key, a nonblank replacement key
   * cancels a pending clear, and a blank key edit leaves a pending clear
   * alone. Turning the clear off never restores a discarded key.
   */
  const updateImageEmbeddingDraft = useCallback((
    patch: Partial<DraftRuntimeSettings["image_embedding"]>,
  ) => {
    runtimeRevision.current += 1;
    setCompatibilityOpen(false);
    setCompatibilityCandidate(null);
    setRuntimeDraft((current) => {
      const next = cloneDraft(current);
      const image = next.image_embedding;
      const { clear_api_key: clearPatch, api_key: keyPatch, ...rest } = patch;
      Object.assign(image, rest);
      if (clearPatch === true) {
        // An explicit clear wins over any key the same patch carries.
        image.clear_api_key = true;
        image.api_key = "";
      } else {
        if (clearPatch === false) {
          // Turning the clear off never restores the discarded key.
          image.clear_api_key = false;
        }
        if (keyPatch !== undefined) {
          image.api_key = keyPatch;
          if (keyPatch.trim() !== "") {
            // Only a nonblank replacement key cancels a pending clear.
            image.clear_api_key = false;
          }
        }
      }
      image.touched = true;
      return next;
    });
  }, []);

  const updateTranslationProviders = useCallback((
    mutate: (providers: TranslationProviderDraft[]) => TranslationProviderDraft[],
  ) => {
    translationRevision.current += 1;
    setTranslationProviders((current) => mutate(current.map((provider) => ({ ...provider }))));
  }, []);

  const runtimeHasChanges = useMemo(() => (
    !!runtimeData
    && JSON.stringify(buildRuntimePayload(runtimeDraft)) !== JSON.stringify(runtimeResponseToPayload(runtimeData))
  ), [runtimeData, runtimeDraft]);

  const doclingHasChanges = useMemo(() => (
    !!doclingData
    && JSON.stringify(buildDoclingPayload(doclingDraft)) !== JSON.stringify(doclingResponseToPayload(doclingData))
  ), [doclingData, doclingDraft]);

  const searchHasChanges = useMemo(() => {
    if (!searchData) {
      return false;
    }
    return JSON.stringify(buildSearchSettingsComparablePayload(searchDraft))
      !== JSON.stringify(searchResponseToPayload(searchData))
      || rerankApiKeyDraft.trim().length > 0;
  }, [searchData, searchDraft, rerankApiKeyDraft]);

  const translationHasChanges = useMemo(() => {
    if (!translationData) {
      return false;
    }
    const savedShape = {
      providers: translationData.providers.map(({ has_api_key: _hasKey, current_month_characters: _usage, ...provider }) => ({
        ...provider,
        api_key: undefined,
      })),
    };
    return JSON.stringify(translationPayload(translationProviders)) !== JSON.stringify(savedShape);
  }, [translationData, translationProviders]);

  const hasChanges = runtimeHasChanges
    || doclingHasChanges
    || searchHasChanges
    || translationHasChanges
    || appearanceDraft.hasChanges;

  const loading = !draftsSeeded
    || runtimeQuery.isPending
    || doclingQuery.isPending
    || searchQuery.isPending
    || (isAdmin && translationQuery.isPending);
  const loadError = runtimeQuery.error ?? doclingQuery.error ?? searchQuery.error ?? translationQuery.error;

  const reload = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ["settings"] });
  }, [queryClient]);

  const emitQdrantOutcome = useCallback((outcome: {
    title: string;
    description?: string;
    duration: number;
    variant: "success" | "error";
  }) => {
    if (qdrantToastId.current !== null) {
      toast.dismiss(qdrantToastId.current);
    }
    qdrantProbeSeq.current += 1;
    const id = `runtime-qdrant-test-${qdrantProbeSeq.current}`;
    qdrantToastId.current = id;
    const options = { id, description: outcome.description, duration: outcome.duration };
    if (outcome.variant === "success") {
      toast.success(outcome.title, options);
    } else {
      toast.error(outcome.title, options);
    }
  }, []);

  const updateQdrantUrl = useCallback((value: string) => {
    qdrantUrlRevision.current += 1;
    if (qdrantToastId.current !== null) {
      toast.dismiss(qdrantToastId.current);
      qdrantToastId.current = null;
    }
    updateRuntimeDraft((draft) => {
      draft.qdrant.url = value;
    });
  }, [updateRuntimeDraft]);

  /**
   * The known preset the lower chunking selection names.
   *
   * A preset is identified by its pinned hash, so a persisted selection
   * renders the preset even before this process prepared it.
   */
  const selectedTokenizerPreset = useMemo(() => {
    if (runtimeDraft.chunking.tokenizer !== "hf_local") {
      return undefined;
    }
    const sha256 = runtimeDraft.chunking.tokenizer_sha256;
    if (!sha256) {
      return undefined;
    }
    return runtimeData?.tokenizer_presets?.find((preset) => preset.sha256 === sha256);
  }, [runtimeData, runtimeDraft.chunking]);

  /**
   * The preparation state of the selected preset.
   *
   * Undefined when the lower selection is not a known preset. A descriptor the
   * serving process reports as ready is ready even before this page prepared
   * it, so a persisted selection renders correctly after a restart.
   */
  const selectedTokenizerStatus = useMemo(() => {
    if (!selectedTokenizerPreset) {
      return undefined;
    }
    const preparation = tokenizerPreparations[selectedTokenizerPreset.id];
    if (preparation) {
      return preparation.status;
    }
    return selectedTokenizerPreset.ready ? "ready" : "idle";
  }, [selectedTokenizerPreset, tokenizerPreparations]);

  /**
   * Whether the lower selection names a preset this process cannot use yet.
   *
   * The section and the shared Save consume this one predicate: a save would
   * otherwise chunk with a tokenizer the serving process does not hold.
   */
  const tokenizerSaveBlocked = selectedTokenizerPreset !== undefined
    && selectedTokenizerStatus !== "ready";

  /**
   * Prepares one known tokenizer preset.
   *
   * The preparation is explicit and bounded: it retrieves the one pinned file
   * of the preset, verifies it, and activates it in the serving process. A
   * repeated click joins the running preparation instead of starting a
   * second one, and an outcome only ever describes the preset it names, so a
   * completion that arrives after the selection moved cannot describe the
   * new choice.
   */
  const prepareTokenizerPreset = useCallback((preset: TokenizerPresetDescriptor) => {
    const running = tokenizerPreparationsInFlight.current.get(preset.id);
    if (running) {
      return running;
    }
    const request = (async () => {
      setTokenizerPreparations((current) => ({
        ...current,
        [preset.id]: { status: "pending" },
      }));
      try {
        await apiClient.provisionTokenizerPreset({ preset_id: preset.id });
        setTokenizerPreparations((current) => ({
          ...current,
          [preset.id]: { status: "ready" },
        }));
        // The readiness belongs to the serving process, so the descriptors
        // are refetched instead of assumed.
        void queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEYS.runtime });
      } catch (error) {
        setTokenizerPreparations((current) => ({
          ...current,
          [preset.id]: {
            status: "error",
            message: errorMessage(error, t("settings.runtime.chunkingTokenizerPresetFailed")),
          },
        }));
      } finally {
        tokenizerPreparationsInFlight.current.delete(preset.id);
      }
    })();
    tokenizerPreparationsInFlight.current.set(preset.id, request);
    return request;
  }, [queryClient, t]);

  /** The preparation state of one known preset. */
  const tokenizerPreparationFor = useCallback(
    (presetId: string): TokenizerPreparation | undefined => tokenizerPreparations[presetId],
    [tokenizerPreparations],
  );

  /**
   * Dispatches one save from the click-time snapshot the native submit
   * carries. `confirmVectorSpace` is set only by the dialog accept handler,
   * so a normal save can never acknowledge a changed gateway/alias on the
   * operator's behalf.
   */
  const performSave = useCallback(async (meta: SettingsSubmitMeta) => {
    const { snapshot, confirmVectorSpace, appearance: appearanceSubmission } = meta;
    setSaveMessage("");

    // Runtime stays the authoritative admission step: its rejection stops
    // every other PUT and the appearance commit.
    const runtimePeer = snapshot.applicable.runtime
      ? defineSettingsSavePeer({
        key: "runtime",
        save: async () => {
          const response = await apiClient.updateRuntimeSettings(
            withVectorSpaceConfirmation(
              buildRuntimePayload(snapshot.runtime),
              confirmVectorSpace,
            ),
          );
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.runtime, response);
          // A committed identity acknowledgement is spent: a later change needs
          // a fresh confirmation.
          setCompatibilityOpen(false);
          setCompatibilityCandidate(null);
          return response;
        },
        adopt: (response) => {
          setRuntimeDraft(runtimeResponseToDraft(response));
          runtimeSeeded.current = runtimeRevision.current;
        },
        isCurrent: () => runtimeRevision.current === snapshot.revisions.runtime,
      })
      : null;

    const doclingPeer = snapshot.applicable.docling
      ? defineSettingsSavePeer({
        key: "doclingSettings",
        save: async () => {
          const response = await apiClient.updateDoclingSettings(
            buildDoclingPayload(snapshot.docling),
          );
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.docling, response);
          return response;
        },
        adopt: (response) => {
          // A programmatic reset is not an operator edit, so it must not move the
          // resource revision.
          form.setFieldValue("doclingSettings", adoptDoclingCommit(response), { dontRunListeners: true });
          doclingSeeded.current = doclingRevision.current;
        },
        isCurrent: () => doclingRevision.current === snapshot.revisions.docling,
      })
      : null;

    const searchPeer = snapshot.applicable.search
      ? defineSettingsSavePeer({
        key: "searchSettings",
        save: async () => {
          const response = await apiClient.updateSearchSettings(
            buildSearchSettingsPayload(snapshot.search, snapshot.searchRerankApiKey, false),
          );
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.search, response);
          return response;
        },
        adopt: (response) => {
          const adopted = adoptSearchCommit(response);
          // A programmatic reset is not an operator edit, so it must not move
          // the resource revision.
          form.setFieldValue("searchSettings", adopted.search, { dontRunListeners: true });
          form.setFieldValue("searchRerankApiKey", adopted.searchRerankApiKey, { dontRunListeners: true });
          searchSeeded.current = searchRevision.current;
        },
        isCurrent: () => searchRevision.current === snapshot.revisions.search,
      })
      : null;

    const translationPeer = snapshot.applicable.translation
      ? defineSettingsSavePeer({
        key: "translation",
        save: async () => {
          const response = await apiClient.updateTranslationSettings(
            translationPayload(snapshot.translation),
          );
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.translation, response);
          return response;
        },
        adopt: (response) => {
          setTranslationProviders(response.providers.map((provider) => ({
            ...provider,
            api_key: undefined,
          })));
          translationSeeded.current = translationRevision.current;
        },
        isCurrent: () => translationRevision.current === snapshot.revisions.translation,
      })
      : null;

    const outcome = await runSettingsSave({
      admission: runtimePeer,
      peers: [doclingPeer, searchPeer, translationPeer].filter(
        (peer): peer is SettingsSavePeer => peer !== null,
      ),
    });

    if (outcome.admissionFailed) {
      showErrorToast(outcome.failures[0]?.reason, t("settings.saveFailed"));
      return;
    }

    // Every fulfilled peer is adopted, so one rejected group cannot hide the
    // others' committed results; a rejection still ends the save.
    if (outcome.failures.length > 0) {
      for (const failure of outcome.failures) {
        showErrorToast(failure.reason, t("settings.saveFailed"));
      }
      return;
    }

    // Appearance follows only a completely successful service save, and uses
    // the submitted snapshot rather than the live draft.
    if (appearanceSubmission) {
      const result = appearanceDraft.commit(appearanceSubmission);
      if (!result.stored) {
        toast.warning(t("settings.appearance.title"), {
          description: t("settings.appearance.notStoredWarning"),
          duration: 6000,
        });
        return;
      }
    }

    // A newer unsaved edit keeps the page pending, so no global success is
    // claimed for it. Every resource counts, not only the ones this save
    // submitted: one that was clean when the operator clicked Save can be
    // edited while the save is running, and that edit is still unsaved.
    const pendingEdit = settingsSaveHasPendingEdit({
      baseline: snapshot.revisions,
      current: {
        runtime: runtimeRevision.current,
        docling: doclingRevision.current,
        search: searchRevision.current,
        translation: translationRevision.current,
      },
      appearanceBaseline: meta.appearanceAtClick,
      appearance: appearanceDraft.captureSnapshot(),
    })
    // An appearance the browser could not store before this save stays
    // pending as well, so no global success hides it.
    || (meta.appearance === null && meta.appearanceUnresolved);

    if (!pendingEdit) {
      setSaveMessage(t("settings.saveSuccess"));
      toast.success(t("settings.saveSuccess"), { duration: 2500 });
    }
  }, [appearanceDraft, form, queryClient, showErrorToast, t]);

  // The native form owns the submit, so the page hands its handler to the
  // form instead of rebuilding the form for every render.
  useEffect(() => {
    submitHandlerRef.current = performSave;
  });

  /**
   * Hands one save to the native form.
   *
   * The click-time snapshot is captured here, before the form submits, so a
   * later edit can neither change a dispatched body nor be overwritten by a
   * late response. A second click while a save runs is refused instead of
   * queued.
   */
  const submitSettings = useCallback((confirmVectorSpace: boolean) => {
    if (dispatchingRef.current) {
      return;
    }
    dispatchingRef.current = true;
    setSaving(true);
    const snapshot = captureSettingsSnapshot({
      runtime: runtimeDraft,
      docling: doclingDraft,
      search: searchDraft,
      searchRerankApiKey: rerankApiKeyDraft,
      translation: translationProviders,
      revisions: {
        runtime: runtimeRevision.current,
        docling: doclingRevision.current,
        search: searchRevision.current,
        translation: translationRevision.current,
      },
      applicable: {
        runtime: runtimeHasChanges,
        docling: doclingHasChanges,
        search: searchHasChanges,
        translation: translationHasChanges,
      },
    });
    const appearance = appearanceDraft.hasChanges ? appearanceDraft.captureSnapshot() : null;
    void form
      .handleSubmit({
        confirmVectorSpace,
        snapshot,
        appearance,
        // The baseline is captured even without appearance changes, so a later
        // edit of an initially clean appearance is recognized as pending.
        appearanceAtClick: appearanceDraft.captureSnapshot(),
        appearanceUnresolved: appearanceDraft.unresolvedCommit,
      })
      .catch((error: unknown) => {
        showErrorToast(error, t("settings.saveFailed"));
      })
      .finally(() => {
        dispatchingRef.current = false;
        setSaving(false);
      });
  }, [
    appearanceDraft,
    doclingDraft,
    doclingHasChanges,
    form,
    rerankApiKeyDraft,
    runtimeDraft,
    runtimeHasChanges,
    searchDraft,
    searchHasChanges,
    showErrorToast,
    t,
    translationHasChanges,
    translationProviders,
  ]);

  const saveSettings = useCallback(async () => {
    if (dispatchingRef.current || !hasChanges) {
      return;
    }
    // A selected preset that is not ready must not be saved: the deployment
    // would chunk with a tokenizer this process cannot use yet.
    if (tokenizerSaveBlocked) {
      toast.error(t("common.error"), {
        description: selectedTokenizerStatus === "pending"
          ? t("settings.runtime.chunkingTokenizerPresetBlocksSavePending")
          : selectedTokenizerStatus === "error"
            ? t("settings.runtime.chunkingTokenizerPresetBlocksSaveFailed")
            : t("settings.runtime.chunkingTokenizerPresetBlocksSaveNotPrepared"),
        duration: 5000,
      });
      return;
    }
    // A rebuild that owns the durable barrier refuses an identity change, so
    // the operator is told to cancel that rebuild instead of meeting a bare
    // conflict. Only the identity is blocked: a chunking or scheduler edit is
    // saved as usual.
    if (isAdmin && runtimeHasChanges && runtimeData) {
      const candidate = buildRuntimePayload(runtimeDraft).embedding;
      const live = runtimeResponseToPayload(runtimeData).embedding;
      if (
        embeddingSignature(candidate) !== embeddingSignature(live)
        && rebuildHoldsBarrier(lastObservedRebuildStatus())
      ) {
        toast.error(t("common.error"), {
          description: t("settings.runtime.vectorRebuildBlocksIdentitySave"),
          duration: 5000,
        });
        return;
      }
    }
    // Gate the whole save on the compatibility decision: a cancelled
    // confirmation must write nothing, not even the other sections.
    if (
      runtimeHasChanges
      && runtimeData
      && needsCompatibilityConfirmation(
        buildRuntimePayload(runtimeDraft).embedding,
        runtimeResponseToPayload(runtimeData).embedding,
      )
    ) {
      setCompatibilityCandidate({
        base_url: runtimeDraft.embedding.base_url.trim(),
        model: runtimeDraft.embedding.model.trim(),
        dimensions: runtimeDraft.embedding.dimensions,
      });
      setCompatibilityOpen(true);
      return;
    }
    submitSettings(false);
  }, [
    hasChanges,
    isAdmin,
    runtimeData,
    runtimeDraft,
    runtimeHasChanges,
    selectedTokenizerStatus,
    submitSettings,
    t,
    tokenizerSaveBlocked,
  ]);
  const confirmCompatibility = useCallback(() => {
    const candidate = compatibilityCandidate;
    setCompatibilityOpen(false);
    setCompatibilityCandidate(null);
    if (!candidate) {
      return;
    }
    // Bound to the candidate the dialog named: an edit made in the meantime
    // invalidates the confirmation instead of acknowledging the new value.
    const live = buildRuntimePayload(runtimeDraft).embedding;
    if (embeddingSignature(live) !== embeddingSignature(candidate)) {
      return;
    }
    submitSettings(true);
  }, [compatibilityCandidate, runtimeDraft, submitSettings]);

  const cancelCompatibility = useCallback(() => {
    setCompatibilityOpen(false);
    setCompatibilityCandidate(null);
  }, []);

  const testS3Connection = useCallback(async () => {
    const s3 = buildRuntimePayload(runtimeDraft).file_library.s3;
    if (!s3) {
      return;
    }
    setS3Testing(true);
    try {
      await apiClient.testS3Connection(s3);
      toast.success(t("settings.runtime.s3TestSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.s3TestFailed"));
    } finally {
      setS3Testing(false);
    }
  }, [runtimeDraft, showErrorToast, t]);

  const testValkeyConnection = useCallback(async () => {
    setValkeyTesting(true);
    try {
      await apiClient.testValkeyConnection({ valkey_url: runtimeDraft.scheduler.valkey_url.trim() });
      toast.success(t("settings.runtime.valkeyTestSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.valkeyTestFailed"));
    } finally {
      setValkeyTesting(false);
    }
  }, [runtimeDraft.scheduler.valkey_url, showErrorToast, t]);

  const testEmbeddingConnection = useCallback(async () => {
    // Snapshot the request once. Both outcomes are presented only while this
    // exact request (including the query alias and credential) is still the
    // observed draft; the revision also covers an edit that lands before the
    // next render updates the signature ref.
    const request = embeddingProbeRequest;
    const requestRevision = runtimeRevision.current;
    const requestSignature = embeddingProbeSignature(request);
    const isStale = () => requestRevision !== runtimeRevision.current
      || embeddingProbeTargetRef.current !== requestSignature;
    setEmbeddingTesting(true);
    try {
      await apiClient.testEmbeddingConnection(request);
      if (isStale()) {
        return;
      }
      toast.success(t("settings.runtime.embeddingTestSuccess"), { duration: 2500 });
    } catch (error) {
      if (isStale()) {
        return;
      }
      showErrorToast(error, t("settings.runtime.embeddingTestFailed"));
    } finally {
      setEmbeddingTesting(false);
    }
  }, [embeddingProbeRequest, showErrorToast, t]);

  /**
   * Probes the unsaved code embedding draft.
   *
   * The probe carries the same sparse overrides a save would store, so a
   * blank field inherits the document configuration and the unsaved document
   * draft travels with the request as the inheritance source.
   */
  const testCodeEmbeddingConnection = useCallback(async () => {
    const request = codeEmbeddingProbeRequest;
    const requestRevision = runtimeRevision.current;
    const requestSignature = embeddingProbeSignature(request);
    const isStale = () => requestRevision !== runtimeRevision.current
      || codeEmbeddingProbeTargetRef.current !== requestSignature;
    setCodeEmbeddingTesting(true);
    try {
      await apiClient.testEmbeddingConnection(request);
      if (isStale()) {
        return;
      }
      toast.success(t("settings.runtime.codeEmbeddingTestSuccess"), { duration: 2500 });
    } catch (error) {
      if (isStale()) {
        return;
      }
      showErrorToast(error, t("settings.runtime.codeEmbeddingTestFailed"));
    } finally {
      setCodeEmbeddingTesting(false);
    }
  }, [codeEmbeddingProbeRequest, showErrorToast, t]);

  /**
   * Probes the unsaved image embedding draft.
   *
   * The probe never saves: it exercises the image route with the draft values
   * only. It embeds one synthetic image, so the query model is carried with
   * the configuration but never exercised by the probe itself. A pending
   * credential clear is not a reason to refuse the probe: the request states
   * the inherited intent, so the probe exercises the document credential that
   * would be in effect after the clear.
   */
  const testImageEmbeddingConnection = useCallback(async () => {
    const request = imageEmbeddingProbeRequest;
    const requestRevision = runtimeRevision.current;
    const requestSignature = embeddingProbeSignature(request);
    const isStale = () => requestRevision !== runtimeRevision.current
      || imageEmbeddingProbeTargetRef.current !== requestSignature;
    setImageEmbeddingTesting(true);
    try {
      await apiClient.testEmbeddingConnection(request);
      if (isStale()) {
        return;
      }
      toast.success(t("settings.runtime.imageEmbeddingTestSuccess"), { duration: 2500 });
    } catch (error) {
      if (isStale()) {
        return;
      }
      showErrorToast(error, t("settings.runtime.imageEmbeddingTestFailed"));
    } finally {
      setImageEmbeddingTesting(false);
    }
  }, [imageEmbeddingProbeRequest, showErrorToast, t]);

  /**
   * Probe the unsaved Qdrant URL. The revision captured at request start must
   * still match when the outcome arrives, so editing the URL away — even back
   * to the same value — invalidates the old outcome.
   */
  const testQdrantConnection = useCallback(async () => {
    if (qdrantTesting) {
      return;
    }
    setQdrantTesting(true);
    const requestRevision = qdrantUrlRevision.current;
    const requestUrl = runtimeDraft.qdrant.url.trim();
    const isStale = () => requestRevision !== qdrantUrlRevision.current
      || requestUrl !== runtimeDraft.qdrant.url.trim();
    try {
      await apiClient.testQdrantConnection({ url: requestUrl });
      if (isStale()) {
        return;
      }
      emitQdrantOutcome({
        variant: "success",
        title: t("settings.runtime.qdrantTestSuccess"),
        duration: 2500,
      });
    } catch (error) {
      if (isStale()) {
        return;
      }
      emitQdrantOutcome({
        variant: "error",
        title: t("common.error"),
        description: errorMessage(error, t("settings.runtime.qdrantTestFailed")),
        duration: 5000,
      });
    } finally {
      setQdrantTesting(false);
    }
  }, [emitQdrantOutcome, qdrantTesting, runtimeDraft.qdrant.url, showErrorToast, t]);

  const testDoclingConnection = useCallback(async () => {
    setDoclingTesting(true);
    try {
      await apiClient.testDoclingConnection(buildDoclingPayload(doclingDraft).connection);
      toast.success(t("settings.docling.testSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.docling.testFailed"));
    } finally {
      setDoclingTesting(false);
    }
  }, [doclingDraft, showErrorToast, t]);

  return {
    appearance: appearanceDraft,
    cancelCompatibility,
    codeEmbeddingTesting,
    compatibilityCandidate,
    compatibilityOpen,
    confirmCompatibility,
    doclingDraft,
    doclingHasChanges,
    doclingQuery,
    doclingTesting,
    embeddingTesting,
    form,
    imageEmbeddingTesting,
    hasChanges,
    isAdmin,
    loadError,
    loading,
    prepareTokenizerPreset,
    qdrantTesting,
    reload,
    rerankApiKeyDraft,
    requiresRebuild,
    runtimeDraft,
    runtimeQuery,
    saveMessage,
    saveSettings,
    saving,
    s3Testing,
    searchDraft,
    searchQuery,
    selectedTokenizerPreset,
    selectedTokenizerStatus,
    testCodeEmbeddingConnection,
    testDoclingConnection,
    testEmbeddingConnection,
    testImageEmbeddingConnection,
    testQdrantConnection,
    testS3Connection,
    testValkeyConnection,
    tokenizerPreparationFor,
    tokenizerSaveBlocked,
    translationHasChanges,
    translationProviders,
    translationQuery,
    updateCodeEmbeddingDraft,
    updateImageEmbeddingDraft,
    updateQdrantUrl,
    updateRuntimeDraft,
    updateTranslationProviders,
    valkeyTesting,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
