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
  TranslationProviderInput,
  TranslationSettingsResponse,
  UpdateRuntimeSettingsRequest,
} from "@/services/api";
import {
  buildDoclingPayload,
  buildRuntimePayload,
  buildSearchSettingsComparablePayload,
  buildSearchSettingsPayload,
  createDoclingDraft,
  createRuntimeDraft,
  createSearchDraft,
  doclingResponseToDraft,
  doclingResponseToPayload,
  runtimeResponseToDraft,
  runtimeResponseToPayload,
  searchResponseToPayload,
  type DraftDoclingSettings,
  type DraftRuntimeSettings,
  type DraftSearchSettings,
} from "@/utils/settings";

import { errorMessage } from "./settings-error";
import { useAppearanceDraft, type AppearanceValues } from "./use-appearance-draft";

export interface TranslationProviderDraft extends Omit<TranslationProviderInput, "enabled"> {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
}

export const SETTINGS_QUERY_KEYS = {
  runtime: ["settings", "runtime"] as const,
  docling: ["settings", "docling"] as const,
  search: ["settings", "search"] as const,
  translation: ["settings", "translation"] as const,
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

function translationPayload(providers: TranslationProviderDraft[]) {
  return {
    providers: providers.map(({ has_api_key: _hasKey, current_month_characters: _usage, ...provider }) => ({
      ...provider,
      api_key: provider.api_key?.trim() || undefined,
    })),
  };
}

function sameAppearance(left: AppearanceValues, right: AppearanceValues) {
  return left.locale === right.locale && left.theme === right.theme && left.timeZone === right.timeZone;
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
  const appearance = useAppearanceDraft();

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
  const [doclingDraft, setDoclingDraft] = useState<DraftDoclingSettings>(createDoclingDraft);
  const [searchDraft, setSearchDraft] = useState<DraftSearchSettings>(createSearchDraft);
  const [rerankApiKeyDraft, setRerankApiKeyDraftState] = useState("");
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

  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const [saveMessage, setSaveMessage] = useState("");

  // A gateway/model change keeps existing vectors only after an explicit
  // confirmation bound to the candidate identity shown in the dialog.
  const [compatibilityOpen, setCompatibilityOpen] = useState(false);
  const [compatibilityCandidate, setCompatibilityCandidate] =
    useState<EmbeddingCompatibilityCandidate | null>(null);

  const [s3Testing, setS3Testing] = useState(false);
  const [valkeyTesting, setValkeyTesting] = useState(false);
  const [embeddingTesting, setEmbeddingTesting] = useState(false);
  const [qdrantTesting, setQdrantTesting] = useState(false);
  const [doclingTesting, setDoclingTesting] = useState(false);
  const qdrantUrlRevision = useRef(0);
  const qdrantToastId = useRef<string | null>(null);
  const qdrantProbeSeq = useRef(0);

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
    setDoclingDraft(doclingResponseToDraft(response));
    doclingSeeded.current = doclingRevision.current;
  }, []);

  const seedSearch = useCallback((response: SearchSettingsResponse) => {
    if (searchRevision.current !== searchSeeded.current) {
      return;
    }
    setSearchDraft(searchResponseToPayload(response));
    searchSeeded.current = searchRevision.current;
  }, []);

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

  const updateDoclingDraft = useCallback((mutate: (draft: DraftDoclingSettings) => void) => {
    doclingRevision.current += 1;
    setDoclingDraft((current) => {
      const next = cloneDraft(current);
      mutate(next);
      return next;
    });
  }, []);

  const updateSearchDraft = useCallback((mutate: (draft: DraftSearchSettings) => void) => {
    searchRevision.current += 1;
    setSearchDraft((current) => {
      const next = cloneDraft(current);
      mutate(next);
      return next;
    });
  }, []);

  const setRerankApiKeyDraft = useCallback((value: string) => {
    searchRevision.current += 1;
    setRerankApiKeyDraftState(value);
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
    || appearance.hasChanges;

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
   * Dispatches one save. `confirmVectorSpace` is set only by the dialog accept
   * handler, so a normal save can never acknowledge a changed gateway/alias on
   * the operator's behalf.
   */
  const performSave = useCallback(async (confirmVectorSpace: boolean) => {
    if (savingRef.current || !hasChanges) {
      return;
    }

    savingRef.current = true;
    setSaving(true);

    try {
      setSaveMessage("");

      // Click-time snapshot: later edits must neither change a dispatched body
      // nor be overwritten by a late response.
      const runtimeSubmission = runtimeHasChanges
        ? {
          payload: withVectorSpaceConfirmation(
            buildRuntimePayload(runtimeDraft),
            confirmVectorSpace,
          ),
          revision: runtimeRevision.current,
        }
        : null;
      const doclingSubmission = doclingHasChanges
        ? { payload: buildDoclingPayload(doclingDraft), revision: doclingRevision.current }
        : null;
      const searchSubmission = searchHasChanges
        ? {
          payload: buildSearchSettingsPayload(searchDraft, rerankApiKeyDraft, false),
          revision: searchRevision.current,
        }
        : null;
      const translationSubmission = translationHasChanges
        ? { payload: translationPayload(translationProviders), revision: translationRevision.current }
        : null;
      const appearanceSubmission = appearance.hasChanges ? appearance.captureSnapshot() : null;

      // Runtime stays the authoritative admission step: its rejection stops
      // every other PUT and the appearance commit.
      if (runtimeSubmission) {
        try {
          const runtime = await apiClient.updateRuntimeSettings(runtimeSubmission.payload);
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.runtime, runtime);
          if (runtimeRevision.current === runtimeSubmission.revision) {
            setRuntimeDraft(runtimeResponseToDraft(runtime));
            runtimeSeeded.current = runtimeRevision.current;
          }
          // A committed identity acknowledgement is spent: a later change needs
          // a fresh confirmation.
          setCompatibilityOpen(false);
          setCompatibilityCandidate(null);
        } catch (error) {
          showErrorToast(error, t("settings.saveFailed"));
          return;
        }
      }

      const [doclingResult, searchResult, translationResult] = await Promise.allSettled([
        doclingSubmission
          ? apiClient.updateDoclingSettings(doclingSubmission.payload)
          : Promise.resolve(null),
        searchSubmission
          ? apiClient.updateSearchSettings(searchSubmission.payload)
          : Promise.resolve(null),
        translationSubmission
          ? apiClient.updateTranslationSettings(translationSubmission.payload)
          : Promise.resolve(null),
      ] as const);

      // Every fulfilled peer is adopted, so one rejected group cannot hide the
      // others' committed results.
      const rejections: unknown[] = [];

      if (doclingResult.status === "fulfilled") {
        if (doclingResult.value) {
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.docling, doclingResult.value);
          if (doclingSubmission && doclingRevision.current === doclingSubmission.revision) {
            setDoclingDraft(doclingResponseToDraft(doclingResult.value));
            doclingSeeded.current = doclingRevision.current;
          }
        }
      } else {
        rejections.push(doclingResult.reason);
      }

      if (searchResult.status === "fulfilled") {
        if (searchResult.value) {
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.search, searchResult.value);
          if (searchSubmission && searchRevision.current === searchSubmission.revision) {
            setSearchDraft(searchResponseToPayload(searchResult.value));
            setRerankApiKeyDraftState("");
            searchSeeded.current = searchRevision.current;
          }
        }
      } else {
        rejections.push(searchResult.reason);
      }

      if (translationResult.status === "fulfilled") {
        if (translationResult.value) {
          queryClient.setQueryData(SETTINGS_QUERY_KEYS.translation, translationResult.value);
          if (translationSubmission && translationRevision.current === translationSubmission.revision) {
            setTranslationProviders(translationResult.value.providers.map((provider) => ({
              ...provider,
              api_key: undefined,
            })));
            translationSeeded.current = translationRevision.current;
          }
        }
      } else {
        rejections.push(translationResult.reason);
      }

      if (rejections.length > 0) {
        for (const reason of rejections) {
          showErrorToast(reason, t("settings.saveFailed"));
        }
        return;
      }

      // Appearance follows only a completely successful service save, and uses
      // the submitted snapshot rather than the live draft.
      let appearancePending = appearance.hasChanges;
      if (appearanceSubmission) {
        const result = appearance.commit(appearanceSubmission);
        const liveAppearance = appearance.captureSnapshot();
        appearancePending = !result.stored || !sameAppearance(liveAppearance, appearanceSubmission);
        if (!result.stored) {
          toast.warning(t("settings.appearance.title"), {
            description: t("settings.appearance.notStoredWarning"),
            duration: 6000,
          });
          return;
        }
      }

      // A newer unsaved edit keeps the page pending, so no global success is
      // claimed for it.
      const stillDirty = appearancePending
        || (!!runtimeSubmission && runtimeRevision.current !== runtimeSubmission.revision)
        || (!!doclingSubmission && doclingRevision.current !== doclingSubmission.revision)
        || (!!searchSubmission && searchRevision.current !== searchSubmission.revision)
        || (!!translationSubmission && translationRevision.current !== translationSubmission.revision);

      if (!stillDirty) {
        setSaveMessage(t("settings.saveSuccess"));
        toast.success(t("settings.saveSuccess"), { duration: 2500 });
      }
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  }, [
    appearance,
    doclingDraft,
    doclingHasChanges,
    hasChanges,
    queryClient,
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
    if (savingRef.current || !hasChanges) {
      return;
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
    await performSave(false);
  }, [hasChanges, performSave, runtimeData, runtimeDraft, runtimeHasChanges]);

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
    void performSave(true);
  }, [compatibilityCandidate, performSave, runtimeDraft]);

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
    setEmbeddingTesting(true);
    try {
      await apiClient.testEmbeddingConnection({
        base_url: runtimeDraft.embedding.base_url.trim(),
        model: runtimeDraft.embedding.model.trim(),
        dimensions: runtimeDraft.embedding.dimensions,
        timeout_secs: runtimeDraft.embedding.timeout_secs,
        api_key: runtimeDraft.embedding.api_key.trim() || undefined,
      });
      toast.success(t("settings.runtime.embeddingTestSuccess"), { duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.embeddingTestFailed"));
    } finally {
      setEmbeddingTesting(false);
    }
  }, [runtimeDraft.embedding, showErrorToast, t]);

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
    appearance,
    cancelCompatibility,
    compatibilityCandidate,
    compatibilityOpen,
    confirmCompatibility,
    doclingDraft,
    doclingHasChanges,
    doclingQuery,
    doclingTesting,
    embeddingTesting,
    hasChanges,
    isAdmin,
    loadError,
    loading,
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
    setRerankApiKeyDraft,
    testDoclingConnection,
    testEmbeddingConnection,
    testQdrantConnection,
    testS3Connection,
    testValkeyConnection,
    translationHasChanges,
    translationProviders,
    translationQuery,
    updateDoclingDraft,
    updateQdrantUrl,
    updateRuntimeDraft,
    updateSearchDraft,
    updateTranslationProviders,
    valkeyTesting,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
