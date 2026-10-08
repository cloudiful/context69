import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";
import { useErrorToast } from "./use-error-toast";

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
import { useSettingsPersonalAccessTokens } from "./use-settings-personal-access-tokens";
import { useSettingsVectorRebuild } from "./use-settings-vector-rebuild";

type TranslationProviderDraft = Omit<TranslationProviderInput, "enabled"> & {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
};

export function useSettingsPage() {
  const { t } = useI18n();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const personalAccessTokens = useSettingsPersonalAccessTokens();
  const adminUsersState = useSettingsAdminUsers();
  const vectorRebuildState = useSettingsVectorRebuild();

  const loading = ref(false);
  const saving = ref(false);
  const s3Testing = ref(false);
  const valkeyTesting = ref(false);
  const embeddingTesting = ref(false);
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
  ));

  watch(hasChanges, (value) => {
    if (value) {
      saveMessage.value = "";
    }
  });

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
    if (!hasChanges.value) {
      return;
    }

    saving.value = true;

    try {
      saveMessage.value = "";
      // The runtime request is the authoritative save admission step: the
      // backend guard rejects an embedding identity change while a fixed vector
      // index is live. Send it first so its rejection surfaces accurately and
      // no other section is dispatched; after it succeeds, the remaining
      // independent section saves keep their existing parallel behavior.
      if (runtimeHasChanges.value) {
        const runtime = await apiClient.updateRuntimeSettings(buildRuntimePayload(runtimeDraft));
        runtimeSettings.value = runtime;
        assignRuntimeDraft(runtime);
      }

      const [docling, search, translation] = await Promise.all([
        doclingHasChanges.value
          ? apiClient.updateDoclingSettings(buildDoclingPayload(doclingDraft))
          : Promise.resolve(doclingSettings.value),
        searchHasChanges.value
          ? apiClient.updateSearchSettings(
            buildSearchSettingsPayload(
              searchDraft,
              rerankApiKeyDraft.value,
              false,
            ),
          )
          : Promise.resolve(searchSettings.value),
        translationHasChanges.value
          ? apiClient.updateTranslationSettings(translationPayload())
          : Promise.resolve(translationSettings.value),
      ]);

      if (docling) {
        doclingSettings.value = docling;
        assignDoclingDraft(docling);
      }
      if (search) {
        searchSettings.value = search;
        assignSearchDraft(search);
      }
      if (translation) {
        translationSettings.value = translation;
        assignTranslationDraft(translation);
      }

      saveMessage.value = t("settings.saveSuccess");
      toast.add({
        color: "success",
        title: t("settings.saveSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.saveFailed"));
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
    doclingDraft,
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
    embeddingTesting,
    translationProviders,
    valkeyTesting,
    vectorRebuildStatus: vectorRebuildState.vectorRebuildStatus,
    confirmVectorIndexRebuild: vectorRebuildState.confirmVectorIndexRebuild,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
