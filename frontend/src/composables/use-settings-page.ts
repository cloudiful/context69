import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";
import { useAppConfirm } from "./use-app-confirm";
import { useErrorToast } from "./use-error-toast";

import {
  apiClient,
  type AdminUserResponse,
  type AdminUserPageResponse,
  type DoclingSettingsResponse,
  type RuntimeSettingsResponse,
  type SearchSettingsResponse,
  type TranslationProviderInput,
  type TranslationSettingsResponse,
  type VectorIndexRebuildStatus,
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
import { useSettingsPersonalAccessTokens } from "./use-settings-personal-access-tokens";

type TranslationProviderDraft = Omit<TranslationProviderInput, "enabled"> & {
  enabled: boolean;
  has_api_key: boolean;
  current_month_characters: number;
};

export function useSettingsPage() {
  const { t } = useI18n();
  const toast = useToast();
  const confirm = useAppConfirm();
  const showErrorToast = useErrorToast();
  const personalAccessTokens = useSettingsPersonalAccessTokens();

  const loading = ref(false);
  const saving = ref(false);
  const s3Testing = ref(false);
  const valkeyTesting = ref(false);
  const vectorRebuildStatus = ref<VectorIndexRebuildStatus | null>(null);
  const saveMessage = ref("");
  const runtimeSettings = ref<RuntimeSettingsResponse | null>(null);
  const doclingSettings = ref<DoclingSettingsResponse | null>(null);
  const searchSettings = ref<SearchSettingsResponse | null>(null);
  const translationSettings = ref<TranslationSettingsResponse | null>(null);
  const translationProviders = ref<TranslationProviderDraft[]>([]);
  const adminUsers = ref<AdminUserResponse[]>([]);
  const adminUsersPage = ref<AdminUserPageResponse>({ items: [], page: 1, page_size: 50, total: 0, total_pages: 0 });
  const adminUsersPageNumber = ref(1);
  const adminUsersPageSize = ref(50);
  const adminUsersQuery = ref("");
  const adminUsersBusy = ref(false);
  const adminUsersCreateBusy = ref(false);
  const rerankApiKeyDraft = ref("");
  let vectorRebuildTimer: ReturnType<typeof setTimeout> | undefined;
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

  async function loadAdminUsers() {
    if (!authSessionState.user?.is_admin) {
      adminUsers.value = [];
      return;
    }

    adminUsersBusy.value = true;
    try {
      const response = await apiClient.listAdminUsers({
        page: adminUsersPageNumber.value,
        page_size: adminUsersPageSize.value,
        query: adminUsersQuery.value.trim() || undefined,
      });
      adminUsersPage.value = response;
      adminUsers.value = response.items;
    } catch (error) {
      showErrorToast(error, t("adminUsers.loadFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  function changeAdminUsersPage(page: number) {
    adminUsersPageNumber.value = page;
    void loadAdminUsers();
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
      await loadVectorIndexRebuildStatus();

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
      const [runtime, docling, search, translation] = await Promise.all([
        runtimeHasChanges.value ? apiClient.updateRuntimeSettings(buildRuntimePayload(runtimeDraft)) : Promise.resolve(runtimeSettings.value),
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

      if (runtime) {
        runtimeSettings.value = runtime;
        assignRuntimeDraft(runtime);
      }
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

  async function loadVectorIndexRebuildStatus() {
    vectorRebuildStatus.value = await apiClient.getVectorIndexRebuildStatus();
    scheduleVectorRebuildPoll();
  }

  function scheduleVectorRebuildPoll() {
    clearTimeout(vectorRebuildTimer);
    if (vectorRebuildStatus.value?.state !== "running") return;
    vectorRebuildTimer = setTimeout(async () => {
      try {
        await loadVectorIndexRebuildStatus();
      } catch (error) {
        showErrorToast(error, t("settings.runtime.vectorRebuildStatusFailed"));
      }
    }, 1500);
  }

  function confirmVectorIndexRebuild() {
    confirm.require({
      header: t("settings.runtime.vectorRebuild"),
      message: t("settings.runtime.vectorRebuildConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("settings.runtime.vectorRebuild"),
      accept: () => void startVectorIndexRebuild(),
    });
  }

  async function startVectorIndexRebuild() {
    try {
      vectorRebuildStatus.value = await apiClient.startVectorIndexRebuild();
      toast.add({ color: "info", title: t("settings.runtime.vectorRebuildStarted"), duration: 2500 });
      scheduleVectorRebuildPoll();
    } catch (error) {
      showErrorToast(error, t("settings.runtime.vectorRebuildFailed"));
    }
  }

  async function createAdminUser(payload: {
    login_name: string;
    display_name: string;
    password: string;
    is_admin: boolean;
  }) {
    adminUsersCreateBusy.value = true;
    try {
      await apiClient.createAdminUser(payload);
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.createFailed"));
    } finally {
      adminUsersCreateBusy.value = false;
    }
  }

  async function updateAdminUser(payload: {
    login_name: string;
    display_name: string;
    is_admin: boolean;
  }) {
    adminUsersBusy.value = true;
    try {
      await apiClient.updateAdminUser(payload.login_name, {
        display_name: payload.display_name,
        is_admin: payload.is_admin,
      });
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.updateFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function resetAdminUserPassword(payload: {
    login_name: string;
    password: string;
  }) {
    adminUsersBusy.value = true;
    try {
      await apiClient.resetAdminUserPassword(payload.login_name, {
        password: payload.password,
      });
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.resetFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function disableAdminUser(loginName: string) {
    adminUsersBusy.value = true;
    try {
      await apiClient.disableAdminUser(loginName);
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.disableFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function enableAdminUser(loginName: string) {
    adminUsersBusy.value = true;
    try {
      await apiClient.enableAdminUser(loginName);
      await loadAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.enableFailed"));
    } finally {
      adminUsersBusy.value = false;
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
    void loadAdminUsers();
    void personalAccessTokens.loadPersonalAccessTokens();
  });

  watch(adminUsersQuery, () => {
    clearTimeout(adminUsersSearchTimer);
    adminUsersSearchTimer = setTimeout(() => {
      adminUsersPageNumber.value = 1;
      void loadAdminUsers();
    }, 250);
  });

  onBeforeUnmount(() => {
    clearTimeout(vectorRebuildTimer);
    clearTimeout(adminUsersSearchTimer);
  });

  return {
    adminUsers,
    adminUsersBusy,
    adminUsersCreateBusy,
    adminUsersPage,
    adminUsersPageNumber,
    adminUsersPageSize,
    adminUsersQuery,
    changeAdminUsersPage,
    createAdminUser,
    doclingDraft,
    disableAdminUser,
    enableAdminUser,
    hasChanges,
    loading,
    qdrantToggleModel,
    rerankApiKeyDraft,
    rerankToggleModel,
    resetAdminUserPassword,
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
    translationProviders,
    updateAdminUser,
    valkeyTesting,
    vectorRebuildStatus,
    confirmVectorIndexRebuild,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
