import { computed, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";
import { useAppConfirm } from "./use-app-confirm";

import {
  apiClient,
  type CreatePersonalAccessTokenResponse,
  type PersonalAccessTokenResponse,
  type PersonalAccessTokenScope,
} from "../services/api";
import { useErrorToast } from "./use-error-toast";

const DEFAULT_SCOPES: PersonalAccessTokenScope[] = ["search"];
const DEFAULT_EXPIRY_DAYS = 30;

export function useSettingsPersonalAccessTokens() {
  const { t } = useI18n();
  const confirm = useAppConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();

  const personalAccessTokens = ref<PersonalAccessTokenResponse[]>([]);
  const personalAccessTokensPage = ref(1);
  const personalAccessTokensPageSize = ref(50);
  const personalAccessTokensTotal = ref(0);
  const personalAccessTokensLoading = ref(false);
  const personalAccessTokensCreating = ref(false);
  const personalAccessTokensReveal = ref<CreatePersonalAccessTokenResponse | null>(null);

  const personalAccessTokenDraft = reactive<{
    name: string;
    scopes: PersonalAccessTokenScope[];
    expires_in_days: number;
  }>({
    name: "",
    scopes: [...DEFAULT_SCOPES],
    expires_in_days: DEFAULT_EXPIRY_DAYS,
  });

  const personalAccessTokenScopeOptions = computed(() => [
    { key: "search", label: t("settings.personalAccessTokens.scopeSearch"), helper: t("settings.personalAccessTokens.scopeSearchHelper") },
    { key: "workspace", label: t("settings.personalAccessTokens.scopeWorkspace"), helper: t("settings.personalAccessTokens.scopeWorkspaceHelper") },
    { key: "library", label: t("settings.personalAccessTokens.scopeLibrary"), helper: t("settings.personalAccessTokens.scopeLibraryHelper") },
    { key: "sources", label: t("settings.personalAccessTokens.scopeSources"), helper: t("settings.personalAccessTokens.scopeSourcesHelper") },
    { key: "settings", label: t("settings.personalAccessTokens.scopeSettings"), helper: t("settings.personalAccessTokens.scopeSettingsHelper") },
    { key: "admin", label: t("settings.personalAccessTokens.scopeAdmin"), helper: t("settings.personalAccessTokens.scopeAdminHelper") },
  ]);

  const personalAccessTokenExpiryOptions = computed(() => [
    { label: t("settings.personalAccessTokens.expiry7Days"), value: 7 },
    { label: t("settings.personalAccessTokens.expiry30Days"), value: 30 },
    { label: t("settings.personalAccessTokens.expiry90Days"), value: 90 },
    { label: t("settings.personalAccessTokens.expiry365Days"), value: 365 },
  ]);

  const personalAccessTokenScopeToggleModel = computed<Record<string, boolean>>({
    get: () => Object.fromEntries(
      personalAccessTokenScopeOptions.value.map((scope) => [
        scope.key,
        personalAccessTokenDraft.scopes.includes(scope.key as PersonalAccessTokenScope),
      ]),
    ),
    set: (value) => {
      personalAccessTokenDraft.scopes = personalAccessTokenScopeOptions.value
        .filter((scope) => !!value[scope.key])
        .map((scope) => scope.key as PersonalAccessTokenScope);
    },
  });

  const personalAccessTokenValidationError = computed(() => {
    if (!personalAccessTokenDraft.name.trim()) {
      return t("settings.personalAccessTokens.nameRequired");
    }
    if (personalAccessTokenDraft.scopes.length === 0) {
      return t("settings.personalAccessTokens.scopesRequired");
    }
    return "";
  });

  const personalAccessTokenCanCreate = computed(() => !personalAccessTokenValidationError.value);

  async function loadPersonalAccessTokens() {
    personalAccessTokensLoading.value = true;
    try {
      const response = await apiClient.listPersonalAccessTokens({
        page: personalAccessTokensPage.value,
        pageSize: personalAccessTokensPageSize.value,
      });
      personalAccessTokens.value = response.items;
      personalAccessTokensTotal.value = response.total;
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.loadFailed"));
    } finally {
      personalAccessTokensLoading.value = false;
    }
  }

  function changePersonalAccessTokensPage(page: number) {
    personalAccessTokensPage.value = page;
    void loadPersonalAccessTokens();
  }

  function changePersonalAccessTokensPageSize(value: number) {
    if (personalAccessTokensPageSize.value === value) return;
    personalAccessTokensPageSize.value = value;
    personalAccessTokensPage.value = 1;
    void loadPersonalAccessTokens();
  }

  async function createPersonalAccessToken() {
    if (personalAccessTokenValidationError.value) {
      showErrorToast(null, personalAccessTokenValidationError.value);
      return;
    }

    personalAccessTokensCreating.value = true;
    try {
      personalAccessTokensReveal.value = await apiClient.createPersonalAccessToken({
        name: personalAccessTokenDraft.name.trim(),
        scopes: personalAccessTokenDraft.scopes,
        expires_in_days: personalAccessTokenDraft.expires_in_days,
      });
      await loadPersonalAccessTokens();
      resetPersonalAccessTokenDraft();
      toast.add({
        color: "success",
        title: t("settings.personalAccessTokens.createSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.createFailed"));
    } finally {
      personalAccessTokensCreating.value = false;
    }
  }

  function confirmRevokePersonalAccessToken(token: PersonalAccessTokenResponse) {
    confirm.require({
      header: t("common.delete"),
      message: t("settings.personalAccessTokens.revokeConfirm", { name: token.name }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("settings.personalAccessTokens.revokeAction"),
      accept: () => {
        void revokePersonalAccessToken(token.token_id);
      },
    });
  }

  async function revokePersonalAccessToken(tokenId: string) {
    try {
      await apiClient.revokePersonalAccessToken(tokenId);
      await loadPersonalAccessTokens();
      toast.add({
        color: "success",
        title: t("settings.personalAccessTokens.revokeSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.revokeFailed"));
    }
  }

  async function copyPersonalAccessToken() {
    const token = personalAccessTokensReveal.value?.access_token;
    if (!token) {
      return;
    }

    try {
      if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
        throw new Error(t("settings.personalAccessTokens.copyUnavailable"));
      }
      await navigator.clipboard.writeText(token);
      toast.add({
        color: "success",
        title: t("settings.personalAccessTokens.copySuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.copyFailed"));
    }
  }

  function dismissPersonalAccessTokenReveal() {
    personalAccessTokensReveal.value = null;
  }

  function resetPersonalAccessTokenDraft() {
    personalAccessTokenDraft.name = "";
    personalAccessTokenDraft.scopes = [...DEFAULT_SCOPES];
    personalAccessTokenDraft.expires_in_days = DEFAULT_EXPIRY_DAYS;
  }

  return {
    copyPersonalAccessToken,
    createPersonalAccessToken,
    dismissPersonalAccessTokenReveal,
    loadPersonalAccessTokens,
    personalAccessTokenDraft,
    personalAccessTokenExpiryOptions,
    personalAccessTokenScopeOptions,
    personalAccessTokenScopeToggleModel,
    personalAccessTokenCanCreate,
    personalAccessTokenValidationError,
    personalAccessTokens,
    personalAccessTokensPage,
    personalAccessTokensPageSize,
    personalAccessTokensTotal,
    changePersonalAccessTokensPage,
    changePersonalAccessTokensPageSize,
    personalAccessTokensCreating,
    personalAccessTokensLoading,
    personalAccessTokensReveal,
    confirmRevokePersonalAccessToken,
  };
}
