import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";
import { errorMessage, useErrorToast } from "./use-error-toast";

import {
  apiClient,
  type DoclingSettingsResponse,
  type RuntimeSettingsResponse,
  type SearchSettingsResponse,
  type TranslationProviderInput,
  type TranslationSettingsResponse,
} from "../services/api";
import { authSessionState } from "../services/auth/session";
import {
  type DraftDoclingSettings,
  type DraftSearchSettings,
  type DraftRuntimeSettings,
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
} from "../utils/settings";
import { useSettingsAdminUsers } from "./use-settings-admin-users";
import { useAppearanceDraft } from "./use-appearance-draft";
import { useSettingsPersonalAccessTokens } from "./use-settings-personal-access-tokens";
import { useSettingsVectorRebuild } from "./use-settings-vector-rebuild";
import { useDoclingRemoteTasks } from "./use-docling-remote-tasks";

type TranslationProviderDraft = Omit<TranslationProviderInput, "enabled"> & {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
};

/**
 * Prefix for Qdrant probe toast ids. Each emitted outcome gets a distinct id
 * from the module sequence below because Nuxt UI deletes a removed toast by
 * id after 200ms: reusing one id lets an earlier edit's delayed deletion erase
 * a fresh outcome. Distinct ids isolate old removals from future outcomes.
 */
const QDRANT_PROBE_TOAST_ID_PREFIX = "runtime-qdrant-test-";

let qdrantProbeToastSeq = 0;

export function useSettingsPage() {
  const { t } = useI18n();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const personalAccessTokens = useSettingsPersonalAccessTokens();
  const adminUsersState = useSettingsAdminUsers();
  const vectorRebuildState = useSettingsVectorRebuild();
  const doclingRemoteTasks = useDoclingRemoteTasks();
  const appearance = useAppearanceDraft();

  const loading = ref(false);
  const saving = ref(false);
  const s3Testing = ref(false);
  const valkeyTesting = ref(false);
  const embeddingTesting = ref(false);
  const qdrantTesting = ref(false);
  const doclingTesting = ref(false);
  /**
   * Generation counter for the unsaved Qdrant URL draft. Every edit bumps it,
   * so a probe started before the edit can recognize its outcome as stale even
   * when the URL is edited away and back to the same value.
   */
  const qdrantUrlRevision = ref(0);
  /**
   * Id of the currently displayed Qdrant probe outcome, if any. The URL watch
   * removes only this outcome and clears the pointer; nothing is removed when
   * no outcome is displayed.
   */
  const activeQdrantToastId = ref<string | null>(null);
  const saveMessage = ref("");
  const runtimeSettings = ref<RuntimeSettingsResponse | null>(null);
  const doclingSettings = ref<DoclingSettingsResponse | null>(null);
  const searchSettings = ref<SearchSettingsResponse | null>(null);
  const translationSettings = ref<TranslationSettingsResponse | null>(null);
  const translationProviders = ref<TranslationProviderDraft[]>([]);
  const rerankApiKeyDraft = ref("");
  let adminUsersSearchTimer: ReturnType<typeof setTimeout> | undefined;

  const runtimeDraft = reactive<DraftRuntimeSettings>(createRuntimeDraft());
  const doclingDraft = reactive<DraftDoclingSettings>(createDoclingDraft());
  const searchDraft = reactive<DraftSearchSettings>(createSearchDraft());

  /** Per-group edit counters: a save only adopts a response over an unedited draft. */
  const runtimeRevision = ref(0);
  const doclingRevision = ref(0);
  const searchRevision = ref(0);
  const translationRevision = ref(0);

  // Synchronous so a click-time snapshot always observes the revision of the
  // latest edit, even before the next render flush.
  watch(runtimeDraft, () => { runtimeRevision.value += 1; }, { deep: true, flush: "sync" });
  watch(doclingDraft, () => { doclingRevision.value += 1; }, { deep: true, flush: "sync" });
  watch(searchDraft, () => { searchRevision.value += 1; }, { deep: true, flush: "sync" });
  watch(rerankApiKeyDraft, () => { searchRevision.value += 1; }, { flush: "sync" });
  watch(translationProviders, () => { translationRevision.value += 1; }, { deep: true, flush: "sync" });

  const searchModeOptions = computed(() => [
    { label: t("settings.search.modeHybrid"), value: "hybrid" },
    { label: t("settings.search.modeVector"), value: "vector" },
  ]);
  const qdrantToggleModel = computed({
    get: () => ({ recreate_on_dimension_mismatch: runtimeDraft.qdrant.recreate_on_dimension_mismatch }),
    set: (value: Record<string, boolean>) => {
      runtimeDraft.qdrant.recreate_on_dimension_mismatch = !!value.recreate_on_dimension_mismatch;
    },
  });

  const schedulerToggleModel = computed({
    get: () => ({ run_on_start: runtimeDraft.scheduler.run_on_start }),
    set: (value: Record<string, boolean>) => {
      runtimeDraft.scheduler.run_on_start = !!value.run_on_start;
    },
  });

  const rerankToggleModel = computed({
    get: () => ({ rerank_enabled: searchDraft.rerank_enabled }),
    set: (value: Record<string, boolean>) => {
      searchDraft.rerank_enabled = !!value.rerank_enabled;
    },
  });

  const runtimeHasChanges = computed(() => (
    runtimeSettings.value
      && JSON.stringify(buildRuntimePayload(runtimeDraft)) !== JSON.stringify(runtimeResponseToPayload(runtimeSettings.value))
  ));

  const doclingHasChanges = computed(() => (
    doclingSettings.value
      && JSON.stringify(buildDoclingPayload(doclingDraft)) !== JSON.stringify(doclingResponseToPayload(doclingSettings.value))
  ));

  const searchHasChanges = computed(() => {
    if (!searchSettings.value) {
      return false;
    }

    return JSON.stringify(buildSearchSettingsComparablePayload(searchDraft)) !== JSON.stringify(searchResponseToPayload(searchSettings.value))
      || rerankApiKeyDraft.value.trim().length > 0;
  });

  const translationHasChanges = computed(() => {
    if (!translationSettings.value) return false;
    return JSON.stringify(translationPayload()) !== JSON.stringify({
      providers: translationSettings.value.providers.map(({ has_api_key: _, current_month_characters: __, ...provider }) => ({
        ...provider,
        api_key: undefined,
      })),
    });
  });

  const hasChanges = computed(() => !!(
    runtimeHasChanges.value
      || doclingHasChanges.value
      || searchHasChanges.value
      || translationHasChanges.value
      || appearance.hasChanges.value
  ));
  const appearanceHasChanges = computed(() => appearance.hasChanges.value);

  watch(hasChanges, (value) => {
    if (value) {
      saveMessage.value = "";
    }
  });

  watch(
    () => runtimeDraft.qdrant.url,
    () => {
      qdrantUrlRevision.value += 1;
      if (activeQdrantToastId.value !== null) {
        toast.remove(activeQdrantToastId.value);
        activeQdrantToastId.value = null;
      }
    },
  );

  /**
   * Emit one Qdrant probe outcome under a fresh id and track it as the active
   * outcome. The previously tracked outcome, if any, is retired first: at most
   * one owned outcome stays visible, so a later URL edit only has to clear the
   * current one. Ids are never reused, so a pending delayed deletion for a
   * removed older outcome cannot erase this one.
   */
  function emitQdrantOutcome(outcome: {
    color: "success" | "error";
    title: string;
    description?: string;
    duration: number;
  }) {
    if (activeQdrantToastId.value !== null) {
      toast.remove(activeQdrantToastId.value);
    }
    qdrantProbeToastSeq += 1;
    const id = `${QDRANT_PROBE_TOAST_ID_PREFIX}${qdrantProbeToastSeq}`;
    activeQdrantToastId.value = id;
    toast.add({ ...outcome, id });
  }

  async function loadPage() {
    loading.value = true;

    try {
      saveMessage.value = "";
      const [runtime, docling, search, translation] = await Promise.all([
        apiClient.getRuntimeSettings(),
        apiClient.getDoclingSettings(),
        apiClient.getSearchSettings(),
        authSessionState.user?.is_admin ? apiClient.getTranslationSettings() : Promise.resolve(null),
      ]);
      runtimeSettings.value = runtime;
      doclingSettings.value = docling;
      searchSettings.value = search;
      translationSettings.value = translation;

      assignRuntimeDraft(runtime);
      assignDoclingDraft(docling);
      assignSearchDraft(search);
      if (translation) assignTranslationDraft(translation);
      await vectorRebuildState.loadVectorRebuildTask();

    } catch (error) {
      showErrorToast(error, t("settings.loadFailed"));
    } finally {
      loading.value = false;
    }
  }

  async function saveSettings() {
    if (saving.value || !hasChanges.value) {
      return;
    }

    saving.value = true;

    try {
      saveMessage.value = "";

      // Click-time snapshot: later edits must neither change a dispatched body
      // nor be overwritten by a late response.
      const runtimeSubmission = runtimeHasChanges.value
        ? { payload: buildRuntimePayload(runtimeDraft), revision: runtimeRevision.value }
        : null;
      const doclingSubmission = doclingHasChanges.value
        ? { payload: buildDoclingPayload(doclingDraft), revision: doclingRevision.value }
        : null;
      const searchSubmission = searchHasChanges.value
        ? {
          payload: buildSearchSettingsPayload(searchDraft, rerankApiKeyDraft.value, false),
          revision: searchRevision.value,
        }
        : null;
      const translationSubmission = translationHasChanges.value
        ? { payload: translationPayload(), revision: translationRevision.value }
        : null;
      const appearanceSubmission = appearance.hasChanges.value
        ? appearance.captureSnapshot()
        : null;

      // Runtime stays the authoritative admission step: its rejection stops
      // every other PUT and the appearance commit.
      if (runtimeSubmission) {
        try {
          const runtime = await apiClient.updateRuntimeSettings(runtimeSubmission.payload);
          runtimeSettings.value = runtime;
          if (runtimeRevision.value === runtimeSubmission.revision) {
            assignRuntimeDraft(runtime);
          }
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
          doclingSettings.value = doclingResult.value;
          if (doclingSubmission && doclingRevision.value === doclingSubmission.revision) {
            assignDoclingDraft(doclingResult.value);
          }
        }
      } else {
        rejections.push(doclingResult.reason);
      }

      if (searchResult.status === "fulfilled") {
        if (searchResult.value) {
          searchSettings.value = searchResult.value;
          if (searchSubmission && searchRevision.value === searchSubmission.revision) {
            assignSearchDraft(searchResult.value);
          }
        }
      } else {
        rejections.push(searchResult.reason);
      }

      if (translationResult.status === "fulfilled") {
        if (translationResult.value) {
          translationSettings.value = translationResult.value;
          if (translationSubmission && translationRevision.value === translationSubmission.revision) {
            assignTranslationDraft(translationResult.value);
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
      if (appearanceSubmission) {
        const result = appearance.commit(appearanceSubmission);
        if (!result.stored) {
          toast.add({
            color: "warning",
            title: t("settings.appearance.title"),
            description: t("settings.appearance.notStoredWarning"),
            duration: 6000,
          });
          return;
        }
      }

      // A newer unsaved edit keeps the page pending, so no global success is
      // claimed for it.
      if (!hasChanges.value) {
        saveMessage.value = t("settings.saveSuccess");
        toast.add({
          color: "success",
          title: t("settings.saveSuccess"),
          duration: 2500,
        });
      }
    } finally {
      saving.value = false;
    }
  }

  async function testS3Connection() {
    const s3 = buildRuntimePayload(runtimeDraft).file_library.s3;
    if (!s3) return;
    s3Testing.value = true;
    try {
      await apiClient.testS3Connection(s3);
      toast.add({
        color: "success",
        title: t("settings.runtime.s3TestSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.s3TestFailed"));
    } finally {
      s3Testing.value = false;
    }
  }

  async function testValkeyConnection() {
    valkeyTesting.value = true;
    try {
      await apiClient.testValkeyConnection({
        valkey_url: runtimeDraft.scheduler.valkey_url.trim(),
      });
      toast.add({
        color: "success",
        title: t("settings.runtime.valkeyTestSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.valkeyTestFailed"));
    } finally {
      valkeyTesting.value = false;
    }
  }

  async function testEmbeddingConnection() {
    embeddingTesting.value = true;
    try {
      await apiClient.testEmbeddingConnection({
        base_url: runtimeDraft.embedding.base_url.trim(),
        model: runtimeDraft.embedding.model.trim(),
        dimensions: runtimeDraft.embedding.dimensions,
        timeout_secs: runtimeDraft.embedding.timeout_secs,
        api_key: runtimeDraft.embedding.api_key.trim() || undefined,
      });
      toast.add({
        color: "success",
        title: t("settings.runtime.embeddingTestSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.embeddingTestFailed"));
    } finally {
      embeddingTesting.value = false;
    }
  }

  /**
   * Probe the Qdrant endpoint currently in the draft, without saving it.
   *
   * Only the unsaved URL is sent: Qdrant has no API key in the saved settings
   * and the collection is irrelevant to a connection check, so neither travels.
   * The button is disabled while the URL is blank and while a probe is in
   * flight, so a duplicate click cannot start a second request.
   *
   * Stale outcomes are suppressed by draft generation: the revision captured at
   * request start must still match when the outcome arrives, so editing the URL
   * away — even back to the same value — invalidates the old outcome. A prior
   * displayed Qdrant outcome is removed on URL edit under its own distinct id,
   * so the removal cannot erase a later outcome.
   */
  async function testQdrantConnection() {
    if (qdrantTesting.value) {
      return;
    }
    qdrantTesting.value = true;
    const requestRevision = qdrantUrlRevision.value;
    const requestUrl = runtimeDraft.qdrant.url.trim();
    const isStale = () =>
      requestRevision !== qdrantUrlRevision.value
      || requestUrl !== runtimeDraft.qdrant.url.trim();
    try {
      await apiClient.testQdrantConnection({
        url: requestUrl,
      });
      if (isStale()) {
        return;
      }
      emitQdrantOutcome({
        color: "success",
        title: t("settings.runtime.qdrantTestSuccess"),
        duration: 2500,
      });
    } catch (error) {
      if (isStale()) {
        return;
      }
      emitQdrantOutcome({
        color: "error",
        title: t("common.error"),
        description: errorMessage(error, t("settings.runtime.qdrantTestFailed")),
        duration: 5000,
      });
    } finally {
      qdrantTesting.value = false;
    }
  }

  /**
   * Probe the Docling endpoint currently in the draft, without saving it.
   *
   * The submitted block is the same normalized connection payload a save would
   * store, so what is checked is exactly what is on screen; the VLM block is not
   * part of it, and the probe resolves no credential.
   */
  async function testDoclingConnection() {
    doclingTesting.value = true;
    try {
      await apiClient.testDoclingConnection(buildDoclingPayload(doclingDraft).connection);
      toast.add({
        color: "success",
        title: t("settings.docling.testSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.docling.testFailed"));
    } finally {
      doclingTesting.value = false;
    }
  }

  function assignRuntimeDraft(response: RuntimeSettingsResponse) {
    Object.assign(runtimeDraft, runtimeResponseToDraft(response));
  }

  function assignDoclingDraft(response: DoclingSettingsResponse) {
    Object.assign(doclingDraft, doclingResponseToDraft(response));
  }

  function assignSearchDraft(response: SearchSettingsResponse) {
    Object.assign(searchDraft, searchResponseToPayload(response));
    rerankApiKeyDraft.value = "";
  }

  function assignTranslationDraft(response: TranslationSettingsResponse) {
    translationProviders.value = response.providers.map((provider) => ({
      ...provider,
      api_key: undefined,
    }));
  }

  function translationPayload() {
    return {
      providers: translationProviders.value.map(({ has_api_key: _, current_month_characters: __, ...provider }) => ({
        ...provider,
        api_key: provider.api_key?.trim() || undefined,
      })),
    };
  }

  onMounted(() => {
    void loadPage();
    void adminUsersState.loadAdminUsers();
    void personalAccessTokens.loadPersonalAccessTokens();
  });

  watch(adminUsersState.adminUsersQuery, () => {
    clearTimeout(adminUsersSearchTimer);
    adminUsersSearchTimer = setTimeout(() => {
      adminUsersState.adminUsersPageNumber.value = 1;
      void adminUsersState.loadAdminUsers();
    }, 250);
  });

  onBeforeUnmount(() => {
    vectorRebuildState.clearVectorRebuildPoll();
    clearTimeout(adminUsersSearchTimer);
  });

  return {
    ...adminUsersState,
    ...doclingRemoteTasks,
    appearanceDraft: appearance.draft,
    appearanceHasChanges,
    doclingDraft,
    doclingTesting,
    hasChanges,
    loading,
    qdrantToggleModel,
    rerankApiKeyDraft,
    rerankToggleModel,
    ...personalAccessTokens,
    saveMessage,
    saveSettings,
    saving,
    s3Testing,
    schedulerToggleModel,
    searchDraft,
    searchModeOptions,
    runtimeDraft,
    testS3Connection,
    testValkeyConnection,
    testEmbeddingConnection,
    testQdrantConnection,
    testDoclingConnection,
    updateAppearanceLocale: appearance.updateLocale,
    updateAppearanceTheme: appearance.updateTheme,
    updateAppearanceTimeZone: appearance.updateTimeZone,
    embeddingTesting,
    qdrantTesting,
    translationProviders,
    valkeyTesting,
    vectorRebuildStatus: vectorRebuildState.vectorRebuildStatus,
    confirmVectorIndexRebuild: vectorRebuildState.confirmVectorIndexRebuild,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
