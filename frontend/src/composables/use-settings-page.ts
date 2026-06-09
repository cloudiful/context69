import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useConfirm } from "primevue/useconfirm";
import { useToast } from "primevue/usetoast";

import {
  apiClient,
  type AdminUserResponse,
  type DoclingSettingsResponse,
  type ProviderAccountResponse,
  type RuntimeSettingsResponse,
  type SearchSettingsResponse,
} from "../services/api";
import { authSessionState } from "../services/auth";
import {
  type DraftDoclingSettings,
  type DraftSearchSettings,
  type DraftRuntimeSettings,
  type ProviderAccountDraft,
  type SettingsNavGroup,
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
import { clearSearchHistory, readSearchHistory } from "../utils/search-history";

const OCR_ENGINES = ["auto", "easyocr", "kserve_v2_ocr", "ocrmac", "rapidocr", "tesserocr", "tesseract"];
const PDF_BACKENDS = ["pypdfium2", "docling_parse", "dlparse_v1", "dlparse_v2", "dlparse_v4"];
const IMAGE_EXPORT_MODES = ["placeholder", "embedded", "referenced"];
const providerKindOptions = [{ label: "openai_compatible", value: "openai_compatible" }];
const timeoutPresets = [60, 120, 300];
const pollPresets = [2, 5, 10];

export function useSettingsPage() {
  const { t } = useI18n();
  const confirm = useConfirm();
  const toast = useToast();

  const loading = ref(false);
  const saving = ref(false);
  const providerSaving = ref(false);
  const pageError = ref("");
  const saveMessage = ref("");
  const providerMessage = ref("");
  const runtimeSettings = ref<RuntimeSettingsResponse | null>(null);
  const doclingSettings = ref<DoclingSettingsResponse | null>(null);
  const searchSettings = ref<SearchSettingsResponse | null>(null);
  const providerAccounts = ref<ProviderAccountResponse[]>([]);
  const recentSearchCount = ref(0);
  const adminUsers = ref<AdminUserResponse[]>([]);
  const adminUsersError = ref("");
  const adminUsersBusy = ref(false);
  const adminUsersCreateBusy = ref(false);
  const ocrLangText = ref("");
  const selectedProviderAccountKey = ref("");
  const rerankApiKeyDraft = ref("");
  const clearStoredRerankApiKey = ref(false);
  const activeSectionId = ref("");

  const runtimeDraft = reactive<DraftRuntimeSettings>(createRuntimeDraft());
  const doclingDraft = reactive<DraftDoclingSettings>(createDoclingDraft());
  const searchDraft = reactive<DraftSearchSettings>(createSearchDraft());
  const providerDraft = reactive<ProviderAccountDraft>(createProviderAccountDraft());

  const settingsNavGroups = computed<SettingsNavGroup[]>(() => {
    const groups: SettingsNavGroup[] = [
      {
        key: "search",
        label: t("settings.search.title"),
        items: [
          { id: "settings-search-history", label: t("settings.search.historyTitle") },
          { id: "settings-search", label: t("settings.search.settingsTitle") },
        ],
      },
      {
        key: "runtime",
        label: t("settings.runtime.title"),
        items: [
          { id: "settings-provider-accounts", label: t("settings.runtime.providerAccountsTitle") },
          { id: "settings-embedding", label: t("settings.runtime.embeddingTitle") },
          { id: "settings-qdrant", label: t("settings.runtime.qdrantTitle") },
          { id: "settings-scheduler", label: t("settings.runtime.schedulerTitle") },
          { id: "settings-chunking", label: t("settings.runtime.chunkingTitle") },
          { id: "settings-file-library", label: t("settings.runtime.fileLibraryTitle") },
        ],
      },
      {
        key: "docling",
        label: t("settings.docling.title"),
        items: [
          { id: "settings-connection", label: t("settings.docling.connectionTitle") },
          { id: "settings-ocr", label: t("settings.docling.ocrTitle") },
          { id: "settings-conversion", label: t("settings.docling.conversionTitle") },
          { id: "settings-enrichment", label: t("settings.docling.enrichmentTitle") },
        ],
      },
    ];

    if (authSessionState.user?.is_admin) {
      groups.push({
        key: "admin",
        label: t("adminUsers.navGroup"),
        items: [
          { id: "settings-admin-users", label: t("adminUsers.title") },
        ],
      });
    }

    return groups;
  });

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

  const timeoutPresetOptions = computed(() => timeoutPresets.map((value) => ({ label: `${value}s`, value })));
  const pollPresetOptions = computed(() => pollPresets.map((value) => ({ label: `${value}s`, value })));

  const selectedTimeoutPreset = computed({
    get: () => (timeoutPresets.includes(doclingDraft.connection.timeout_secs) ? doclingDraft.connection.timeout_secs : null),
    set: (value: number | null) => {
      if (typeof value === "number") {
        doclingDraft.connection.timeout_secs = value;
      }
    },
  });

  const selectedPollPreset = computed({
    get: () => (pollPresets.includes(doclingDraft.connection.poll_interval_secs) ? doclingDraft.connection.poll_interval_secs : null),
    set: (value: number | null) => {
      if (typeof value === "number") {
        doclingDraft.connection.poll_interval_secs = value;
      }
    },
  });

  const ocrEngineOptions = computed(() => [
    { label: t("settings.docling.useServiceDefault"), value: "" },
    ...OCR_ENGINES.map((value) => ({ label: value, value })),
  ]);

  const pdfBackendOptions = computed(() => [
    { label: t("settings.docling.useServiceDefault"), value: "" },
    ...PDF_BACKENDS.map((value) => ({ label: value, value })),
  ]);

  const imageExportModeOptions = computed(() => [
    { label: t("settings.docling.useServiceDefault"), value: "" },
    ...IMAGE_EXPORT_MODES.map((value) => ({ label: value, value })),
  ]);

  const searchModeOptions = computed(() => [
    { label: t("settings.search.modeHybrid"), value: "hybrid" },
    { label: t("settings.search.modeVector"), value: "vector" },
  ]);
  const searchHasStoredApiKey = computed(() => !!searchSettings.value?.has_api_key);

  function scrollToSettingsSection(sectionId: string) {
    const section = document.getElementById(sectionId);
    if (!section) {
      return;
    }

    activeSectionId.value = sectionId;
    if (typeof section.scrollIntoView === "function") {
      section.scrollIntoView({
        block: "start",
        behavior: "smooth",
      });
    }
    window.history.replaceState(null, "", `#${sectionId}`);
  }

  function initializeActiveSection() {
    const sectionIds = settingsNavGroups.value.flatMap((group) => group.items.map((item) => item.id));
    const hashSectionId = window.location.hash.replace(/^#/, "");
    activeSectionId.value = sectionIds.includes(hashSectionId) ? hashSectionId : (sectionIds[0] ?? "");
  }

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

  const ocrToggleModel = computed({
    get: () => ({
      do_ocr: doclingDraft.ocr.do_ocr,
      force_ocr: doclingDraft.ocr.force_ocr,
    }),
    set: (value: Record<string, boolean>) => {
      doclingDraft.ocr.do_ocr = !!value.do_ocr;
      doclingDraft.ocr.force_ocr = !!value.force_ocr;
    },
  });

  const enrichmentToggleModel = computed({
    get: () => ({
      do_code_enrichment: doclingDraft.enrichment.do_code_enrichment,
      do_formula_enrichment: doclingDraft.enrichment.do_formula_enrichment,
      do_picture_description: doclingDraft.enrichment.do_picture_description,
    }),
    set: (value: Record<string, boolean>) => {
      doclingDraft.enrichment.do_code_enrichment = !!value.do_code_enrichment;
      doclingDraft.enrichment.do_formula_enrichment = !!value.do_formula_enrichment;
      doclingDraft.enrichment.do_picture_description = !!value.do_picture_description;
    },
  });

  const rerankToggleModel = computed({
    get: () => ({ rerank_enabled: searchDraft.rerank_enabled }),
    set: (value: Record<string, boolean>) => {
      searchDraft.rerank_enabled = !!value.rerank_enabled;
    },
  });

  const rerankApiKeyToggleModel = computed({
    get: () => ({ clear_api_key: clearStoredRerankApiKey.value }),
    set: (value: Record<string, boolean>) => {
      clearStoredRerankApiKey.value = !!value.clear_api_key;
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
      && JSON.stringify(buildDoclingPayload(doclingDraft, ocrLangText.value)) !== JSON.stringify(doclingResponseToPayload(doclingSettings.value))
  ));

  const searchHasChanges = computed(() => {
    if (!searchSettings.value) {
      return false;
    }

    return JSON.stringify(buildSearchSettingsComparablePayload(searchDraft)) !== JSON.stringify(searchResponseToPayload(searchSettings.value))
      || rerankApiKeyDraft.value.trim().length > 0
      || clearStoredRerankApiKey.value;
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

  watch(rerankApiKeyDraft, (value) => {
    if (value.trim()) {
      clearStoredRerankApiKey.value = false;
    }
  });

  function reloadSearchHistoryState() {
    recentSearchCount.value = readSearchHistory().length;
  }

  function clearRecentSearches() {
    clearSearchHistory();
    reloadSearchHistoryState();
  }

  async function loadAdminUsers() {
    if (!authSessionState.user?.is_admin) {
      adminUsers.value = [];
      adminUsersError.value = "";
      return;
    }

    adminUsersBusy.value = true;
    try {
      adminUsersError.value = "";
      adminUsers.value = await apiClient.listAdminUsers();
    } catch (error) {
      adminUsersError.value = error instanceof Error ? error.message : t("adminUsers.loadFailed");
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function loadPage() {
    loading.value = true;

    try {
      pageError.value = "";
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
      pageError.value = error instanceof Error ? error.message : t("settings.loadFailed");
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
      pageError.value = "";
      saveMessage.value = "";
      providerMessage.value = "";

      if (providerHasChanges.value) {
        await persistProviderAccount();
      }

      const [runtime, docling, search] = await Promise.all([
        runtimeHasChanges.value ? apiClient.updateRuntimeSettings(buildRuntimePayload(runtimeDraft)) : Promise.resolve(runtimeSettings.value),
        doclingHasChanges.value
          ? apiClient.updateDoclingSettings(buildDoclingPayload(doclingDraft, ocrLangText.value))
          : Promise.resolve(doclingSettings.value),
        searchHasChanges.value
          ? apiClient.updateSearchSettings(
            buildSearchSettingsPayload(
              searchDraft,
              rerankApiKeyDraft.value,
              clearStoredRerankApiKey.value,
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
      pageError.value = error instanceof Error ? error.message : t("settings.saveFailed");
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
      pageError.value = error instanceof Error ? error.message : t("settings.saveFailed");
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
      adminUsersError.value = "";
      await apiClient.createAdminUser(payload);
      await loadAdminUsers();
    } catch (error) {
      adminUsersError.value = error instanceof Error ? error.message : t("adminUsers.createFailed");
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
      adminUsersError.value = "";
      await apiClient.updateAdminUser(payload.login_name, {
        display_name: payload.display_name,
        is_admin: payload.is_admin,
      });
      await loadAdminUsers();
    } catch (error) {
      adminUsersError.value = error instanceof Error ? error.message : t("adminUsers.updateFailed");
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
      adminUsersError.value = "";
      await apiClient.resetAdminUserPassword(payload.login_name, {
        password: payload.password,
      });
      await loadAdminUsers();
    } catch (error) {
      adminUsersError.value = error instanceof Error ? error.message : t("adminUsers.resetFailed");
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function disableAdminUser(loginName: string) {
    adminUsersBusy.value = true;
    try {
      adminUsersError.value = "";
      await apiClient.disableAdminUser(loginName);
      await loadAdminUsers();
    } catch (error) {
      adminUsersError.value = error instanceof Error ? error.message : t("adminUsers.disableFailed");
    } finally {
      adminUsersBusy.value = false;
    }
  }

  async function enableAdminUser(loginName: string) {
    adminUsersBusy.value = true;
    try {
      adminUsersError.value = "";
      await apiClient.enableAdminUser(loginName);
      await loadAdminUsers();
    } catch (error) {
      adminUsersError.value = error instanceof Error ? error.message : t("adminUsers.enableFailed");
    } finally {
      adminUsersBusy.value = false;
    }
  }

  function assignRuntimeDraft(response: RuntimeSettingsResponse) {
    Object.assign(runtimeDraft, runtimeResponseToDraft(response));
  }

  function assignDoclingDraft(response: DoclingSettingsResponse) {
    const { draft, ocrLangText: nextOcrLangText } = doclingResponseToDraft(response);
    Object.assign(doclingDraft, draft);
    ocrLangText.value = nextOcrLangText;
  }

  function assignSearchDraft(response: SearchSettingsResponse) {
    Object.assign(searchDraft, searchResponseToPayload(response));
    rerankApiKeyDraft.value = "";
    clearStoredRerankApiKey.value = false;
  }

  onMounted(() => {
    initializeActiveSection();
    reloadSearchHistoryState();
    void loadPage();
    void loadAdminUsers();
  });

  return {
    activeSectionId,
    adminUsers,
    adminUsersBusy,
    adminUsersCreateBusy,
    adminUsersError,
    clearRecentSearches,
    createAdminUser,
    deleteProviderAccount,
    doclingDraft,
    disableAdminUser,
    enableAdminUser,
    doclingProviderOptions,
    enrichmentToggleModel,
    hasChanges,
    imageExportModeOptions,
    loading,
    ocrEngineOptions,
    ocrLangText,
    ocrToggleModel,
    pageError,
    pdfBackendOptions,
    pollPresetOptions,
    providerAccountOptions,
    providerDraft,
    providerKindOptions,
    providerMessage,
    providerSaving,
    providerStatusLabel,
    providerToggleModel,
    qdrantToggleModel,
    recentSearchCount,
    rerankApiKeyDraft,
    rerankApiKeyToggleModel,
    rerankToggleModel,
    resetAdminUserPassword,
    saveMessage,
    saveSettings,
    saving,
    schedulerToggleModel,
    searchHasStoredApiKey,
    scrollToSettingsSection,
    searchDraft,
    searchModeOptions,
    selectedPollPreset,
    selectedProviderAccount,
    selectedProviderAccountKey,
    selectedTimeoutPreset,
    settingsNavGroups,
    startNewProviderAccount,
    runtimeDraft,
    timeoutPresetOptions,
    toggleClearProviderApiKey,
    updateAdminUser,
  };
}
