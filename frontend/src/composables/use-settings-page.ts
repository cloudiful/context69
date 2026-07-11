import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "primevue/usetoast";
import { useErrorToast } from "./use-error-toast";

import {
  apiClient,
  type AdminUserResponse,
  type DoclingSettingsResponse,
  type RuntimeSettingsResponse,
  type SearchSettingsResponse,
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

export function useSettingsPage() {
  const { t } = useI18n();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const personalAccessTokens = useSettingsPersonalAccessTokens();

  const loading = ref(false);
  const saving = ref(false);
  const saveMessage = ref("");
  const runtimeSettings = ref<RuntimeSettingsResponse | null>(null);
  const doclingSettings = ref<DoclingSettingsResponse | null>(null);
  const searchSettings = ref<SearchSettingsResponse | null>(null);
  const adminUsers = ref<AdminUserResponse[]>([]);
  const adminUsersBusy = ref(false);
  const adminUsersCreateBusy = ref(false);
  const rerankApiKeyDraft = ref("");

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

  const hasChanges = computed(() => !!(
    runtimeHasChanges.value
      || doclingHasChanges.value
      || searchHasChanges.value
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
      adminUsers.value = await apiClient.listAdminUsers();
    } catch (error) {
      showErrorToast(error, t("adminUsers.loadFailed"));
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function loadPage() {
    loading.value = true;

    try {
      saveMessage.value = "";
      const [runtime, docling, search] = await Promise.all([
        apiClient.getRuntimeSettings(),
        apiClient.getDoclingSettings(),
        apiClient.getSearchSettings(),
      ]);
      runtimeSettings.value = runtime;
      doclingSettings.value = docling;
      searchSettings.value = search;

      assignRuntimeDraft(runtime);
      assignDoclingDraft(docling);
      assignSearchDraft(search);

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
      const [runtime, docling, search] = await Promise.all([
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

      saveMessage.value = t("settings.saveSuccess");
      toast.add({
        severity: "success",
        summary: t("settings.saveSuccess"),
        life: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.saveFailed"));
    } finally {
      saving.value = false;
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

  onMounted(() => {
    void loadPage();
    void loadAdminUsers();
    void personalAccessTokens.loadPersonalAccessTokens();
  });

  return {
    adminUsers,
    adminUsersBusy,
    adminUsersCreateBusy,
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
    schedulerToggleModel,
    searchDraft,
    searchModeOptions,
    runtimeDraft,
    updateAdminUser,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
