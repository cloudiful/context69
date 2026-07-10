import { computed, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useConfirm } from "primevue/useconfirm";
import { useToast } from "primevue/usetoast";

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
  const confirm = useConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();

  const personalAccessTokens = ref<PersonalAccessTokenResponse[]>([]);
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
      personalAccessTokens.value = await apiClient.listPersonalAccessTokens();
    } catch (error) {
      showErrorToast(error, t("settings.personalAccessTokens.loadFailed"));
    } finally {
      personalAccessTokensLoading.value = false;
    }
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
      personalAccessTokens.value = await apiClient.listPersonalAccessTokens();
      resetPersonalAccessTokenDraft();
      toast.add({
        severity: "success",
        summary: t("settings.personalAccessTokens.createSuccess"),
        life: 2500,
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
      icon: "pi pi-exclamation-triangle",
      rejectProps: {
        label: t("common.cancel"),
        severity: "secondary",
        outlined: true,
      },
      acceptProps: {
        label: t("settings.personalAccessTokens.revokeAction"),
        severity: "danger",
      },
      accept: () => {
        void revokePersonalAccessToken(token.token_id);
      },
    });
  }

  async function revokePersonalAccessToken(tokenId: string) {
    try {
      await apiClient.revokePersonalAccessToken(tokenId);
      personalAccessTokens.value = await apiClient.listPersonalAccessTokens();
      toast.add({
        severity: "success",
        summary: t("settings.personalAccessTokens.revokeSuccess"),
        life: 2500,
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
        severity: "success",
        summary: t("settings.personalAccessTokens.copySuccess"),
        life: 2500,
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
    personalAccessTokensCreating,
    personalAccessTokensLoading,
    personalAccessTokensReveal,
    confirmRevokePersonalAccessToken,
  };
}
