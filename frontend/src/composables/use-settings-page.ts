import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useConfirm } from "primevue/useconfirm";
import { useToast } from "primevue/usetoast";
import { useErrorToast } from "./use-error-toast";

import {
  apiClient,
  type AdminUserResponse,
  type DoclingSettingsResponse,
  type ProviderAccountResponse,
  type RuntimeSettingsResponse,
  type SearchSettingsResponse,
} from "../services/api";
import { authSessionState } from "../services/auth/session";
import {
  type DraftDoclingSettings,
  type DraftSearchSettings,
  type DraftRuntimeSettings,
  type ProviderAccountDraft,
  buildDoclingPayload,
  buildProviderAccountComparablePayload,
  buildProviderAccountPayload,
  buildRuntimePayload,
  buildSearchSettingsComparablePayload,
  buildSearchSettingsPayload,
  createDoclingDraft,
  createProviderAccountDraft,
  createRuntimeDraft,
  createSearchDraft,
  doclingResponseToDraft,
  doclingResponseToPayload,
  isProviderDraftBlank,
  providerResponseToDraft,
  providerResponseToPayload,
  runtimeResponseToDraft,
  runtimeResponseToPayload,
  searchResponseToPayload,
} from "../utils/settings";
import { useSettingsPersonalAccessTokens } from "./use-settings-personal-access-tokens";

const providerKindOptions = [{ label: "openai_compatible", value: "openai_compatible" }];

export function useSettingsPage() {
  const { t } = useI18n();
  const confirm = useConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const personalAccessTokens = useSettingsPersonalAccessTokens();

  const loading = ref(false);
  const saving = ref(false);
  const providerSaving = ref(false);
  const saveMessage = ref("");
  const providerMessage = ref("");
  const runtimeSettings = ref<RuntimeSettingsResponse | null>(null);
  const doclingSettings = ref<DoclingSettingsResponse | null>(null);
  const searchSettings = ref<SearchSettingsResponse | null>(null);
  const providerAccounts = ref<ProviderAccountResponse[]>([]);
  const adminUsers = ref<AdminUserResponse[]>([]);
  const adminUsersBusy = ref(false);
  const adminUsersCreateBusy = ref(false);
  const selectedProviderAccountKey = ref("");
  const rerankApiKeyDraft = ref("");

  const runtimeDraft = reactive<DraftRuntimeSettings>(createRuntimeDraft());
  const doclingDraft = reactive<DraftDoclingSettings>(createDoclingDraft());
  const searchDraft = reactive<DraftSearchSettings>(createSearchDraft());
  const providerDraft = reactive<ProviderAccountDraft>(createProviderAccountDraft());

  const providerAccountOptions = computed(() => [
    { label: t("settings.runtime.noneSelected"), value: "" },
    ...providerAccounts.value.map((account) => ({
      label: `${account.display_name} (${account.account_key})`,
      value: account.account_key,
    })),
  ]);

  const doclingProviderOptions = computed(() => [
    { label: t("settings.docling.providerOptional"), value: "" },
    ...providerAccounts.value.map((account) => ({
      label: `${account.display_name} (${account.account_key})`,
      value: account.account_key,
    })),
  ]);

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

  const providerToggleModel = computed({
    get: () => ({ disabled: providerDraft.disabled }),
    set: (value: Record<string, boolean>) => {
      providerDraft.disabled = !!value.disabled;
    },
  });

  const rerankToggleModel = computed({
    get: () => ({ rerank_enabled: searchDraft.rerank_enabled }),
    set: (value: Record<string, boolean>) => {
      searchDraft.rerank_enabled = !!value.rerank_enabled;
    },
  });

  const selectedProviderAccount = computed(() => (
    providerAccounts.value.find((account) => account.account_key === selectedProviderAccountKey.value) ?? null
  ));

  const providerStatusLabel = computed(() => {
    if (providerDraft.clear_api_key) {
      return t("settings.runtime.providerApiKeyPendingClear");
    }
    if (selectedProviderAccount.value?.has_api_key) {
      return t("settings.runtime.providerApiKeyStored");
    }
    return t("settings.runtime.providerApiKeyMissing");
  });

  const providerHasChanges = computed(() => {
    if (!selectedProviderAccount.value) {
      return !isProviderDraftBlank(providerDraft);
    }

    return JSON.stringify(buildProviderAccountComparablePayload(providerDraft))
      !== JSON.stringify(providerResponseToPayload(selectedProviderAccount.value))
      || providerDraft.api_key.trim().length > 0;
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
    providerHasChanges.value
      || runtimeHasChanges.value
      || doclingHasChanges.value
      || searchHasChanges.value
  ));

  watch(hasChanges, (value) => {
    if (value) {
      saveMessage.value = "";
    }
  });

  watch(selectedProviderAccountKey, (accountKey) => {
    const account = providerAccounts.value.find((item) => item.account_key === accountKey);
    if (!account) {
      Object.assign(providerDraft, createProviderAccountDraft());
      return;
    }

    Object.assign(providerDraft, providerResponseToDraft(account));
    providerMessage.value = "";
  });

  watch(() => providerDraft.api_key, (value) => {
    if (value.trim()) {
      providerDraft.clear_api_key = false;
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
      const [runtime, docling, search, providers] = await Promise.all([
        apiClient.getRuntimeSettings(),
        apiClient.getDoclingSettings(),
        apiClient.getSearchSettings(),
        apiClient.listProviderAccounts(),
      ]);
      runtimeSettings.value = runtime;
      doclingSettings.value = docling;
      searchSettings.value = search;
      providerAccounts.value = providers;

      assignRuntimeDraft(runtime);
      assignDoclingDraft(docling);
      assignSearchDraft(search);

      if (!selectedProviderAccountKey.value && providers[0]) {
        selectedProviderAccountKey.value = providers[0].account_key;
      }
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
      providerMessage.value = "";

      if (providerHasChanges.value) {
        await persistProviderAccount();
      }

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

  function deleteProviderAccount() {
    if (!selectedProviderAccount.value) {
      return;
    }

    const accountKey = selectedProviderAccount.value.account_key;
    confirm.require({
      header: t("common.delete"),
      message: t("settings.runtime.providerDeleteConfirm", { key: accountKey }),
      icon: "pi pi-exclamation-triangle",
      rejectProps: {
        label: t("common.cancel"),
        severity: "secondary",
        outlined: true,
      },
      acceptProps: {
        label: t("common.delete"),
        severity: "danger",
      },
      accept: () => {
        void deleteProviderAccountConfirmed(accountKey);
      },
    });
  }

  async function deleteProviderAccountConfirmed(accountKey: string) {
    providerSaving.value = true;
    try {
      await apiClient.deleteProviderAccount(accountKey);
      providerAccounts.value = await apiClient.listProviderAccounts();
      selectedProviderAccountKey.value = providerAccounts.value[0]?.account_key ?? "";
      if (!selectedProviderAccountKey.value) {
        Object.assign(providerDraft, createProviderAccountDraft());
      }
      providerMessage.value = t("settings.runtime.providerDeleteSuccess");
      toast.add({
        severity: "success",
        summary: t("settings.runtime.providerDeleteSuccess"),
        detail: accountKey,
        life: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.saveFailed"));
    } finally {
      providerSaving.value = false;
    }
  }

  function startNewProviderAccount() {
    selectedProviderAccountKey.value = "";
    Object.assign(providerDraft, createProviderAccountDraft());
    providerMessage.value = "";
  }

  function toggleClearProviderApiKey() {
    if (!selectedProviderAccount.value?.has_api_key && !providerDraft.clear_api_key) {
      return;
    }
    providerDraft.clear_api_key = !providerDraft.clear_api_key;
    if (providerDraft.clear_api_key) {
      providerDraft.api_key = "";
    }
  }

  async function persistProviderAccount() {
    const payload = buildProviderAccountPayload(providerDraft);
    const exists = !!selectedProviderAccount.value;
    const saved = exists
      ? await apiClient.updateProviderAccount(payload)
      : await apiClient.createProviderAccount(payload);
    providerAccounts.value = await apiClient.listProviderAccounts();
    selectedProviderAccountKey.value = saved.account_key;
    Object.assign(providerDraft, providerResponseToDraft(saved));
    return saved;
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
    deleteProviderAccount,
    doclingDraft,
    disableAdminUser,
    enableAdminUser,
    doclingProviderOptions,
    hasChanges,
    loading,
    providerAccountOptions,
    providerDraft,
    providerKindOptions,
    providerMessage,
    providerSaving,
    providerStatusLabel,
    providerToggleModel,
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
    selectedProviderAccount,
    selectedProviderAccountKey,
    startNewProviderAccount,
    runtimeDraft,
    toggleClearProviderApiKey,
    updateAdminUser,
  };
}

export type SettingsPageState = ReturnType<typeof useSettingsPage>;
