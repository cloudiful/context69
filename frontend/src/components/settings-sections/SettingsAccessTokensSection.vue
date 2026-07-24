<script setup lang="ts">
import { computed } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import AppSelectField from "../AppSelectField.vue";
import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import TablePagination from "../TablePagination.vue";
import type {
  CreatePersonalAccessTokenResponse,
  PersonalAccessTokenResponse,
  Pagination,
} from "../../services/api";

const props = defineProps<{
  createPersonalAccessToken: () => Promise<void>;
  confirmRevokePersonalAccessToken: (token: PersonalAccessTokenResponse) => void;
  copyPersonalAccessToken: () => Promise<void>;
  dismissPersonalAccessTokenReveal: () => void;
  personalAccessTokenDraft: {
    name: string;
    scopes: string[];
    expires_in_days: number;
  };
  personalAccessTokenCanCreate: boolean;
  personalAccessTokenExpiryOptions: Array<{ label: string; value: number }>;
  personalAccessTokenScopeOptions: Array<{ key: string; label: string; helper: string }>;
  personalAccessTokenScopeToggleModel: Record<string, boolean>;
  personalAccessTokens: PersonalAccessTokenResponse[];
  personalAccessTokensPagination: Pagination;
  personalAccessTokensCreating: boolean;
  personalAccessTokensLoading: boolean;
  personalAccessTokensReveal: CreatePersonalAccessTokenResponse | null;
}>();

const emit = defineEmits<{
  "update:personalAccessTokenScopeToggleModel": [value: Record<string, boolean>];
  page: [number];
  "page-size": [number];
}>();

const { t, locale } = useI18n();

const tokenRows = computed(() => props.personalAccessTokens.map((token) => ({
  ...token,
  scopeSummary: token.scopes.map((scope) => t(`settings.personalAccessTokens.scopeLabel.${scope}`)).join(", "),
  expiresLabel: formatDate(token.expires_at),
  createdLabel: formatDate(token.created_at),
  lastUsedLabel: token.last_used_at ? formatDate(token.last_used_at) : t("settings.personalAccessTokens.neverUsed"),
  status: resolveStatus(token),
})));
function formatDate(value: string) {
  return new Date(value).toLocaleString(locale.value);
}

function resolveStatus(token: PersonalAccessTokenResponse) {
  if (token.revoked_at) {
    return {
      label: t("settings.personalAccessTokens.statusRevoked"),
      color: "error" as const,
    };
  }
  if (new Date(token.expires_at).getTime() <= Date.now()) {
    return {
      label: t("settings.personalAccessTokens.statusExpired"),
      color: "warning" as const,
    };
  }
  return {
    label: t("settings.personalAccessTokens.statusActive"),
    color: "success" as const,
  };
}

type TokenRow = (typeof tokenRows.value)[number];
const columns = computed<TableColumn<TokenRow>[]>(() => [
  { accessorKey: "name", header: t("settings.personalAccessTokens.name") },
  { accessorKey: "scopeSummary", header: t("settings.personalAccessTokens.scopes") },
  { accessorKey: "createdLabel", header: t("settings.personalAccessTokens.createdAt") },
  { accessorKey: "expiresLabel", header: t("settings.personalAccessTokens.expiresAt") },
  { accessorKey: "lastUsedLabel", header: t("settings.personalAccessTokens.lastUsedAt") },
  { id: "status", header: t("settings.personalAccessTokens.status") },
  { id: "actions", header: t("settings.personalAccessTokens.actions") },
]);

function updateScopeToggleModel(value: Record<string, boolean>) {
  emit("update:personalAccessTokenScopeToggleModel", value);
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.personalAccessTokens.title')">
    <div class="grid gap-4">
      <AppSettingsBlock compact>
        <div class="grid gap-3">
          <AppToggleGroup
            :model-value="personalAccessTokenScopeToggleModel"
            columns-class="grid max-w-2xl gap-y-2 [&_.app-toggle-field]:min-h-8 [&_.app-toggle-field]:px-2.5 [&_.app-toggle-field]:py-1.5"
            helper-inline
            :items="personalAccessTokenScopeOptions.map((scope) => ({
              key: scope.key,
              inputId: `pat-scope-${scope.key}`,
              label: scope.label,
              helper: scope.helper,
              testId: `pat-scope-${scope.key}`,
            }))"
            @update:model-value="updateScopeToggleModel"
          />

          <div class="grid gap-2 md:grid-cols-[minmax(0,1fr)_11rem_auto] md:items-start">
            <AppTextField
              input-id="personal-access-token-name"
              v-model="personalAccessTokenDraft.name"
              :label="t('settings.personalAccessTokens.name')"
              test-id="personal-access-token-name"
            />
            <AppSelectField
              input-id="personal-access-token-expiry"
              v-model="personalAccessTokenDraft.expires_in_days"
              :label="t('settings.personalAccessTokens.expiry')"
              :options="personalAccessTokenExpiryOptions"
              test-id="personal-access-token-expiry"
            />
            <div class="flex items-end">
              <UButton
                data-testid="personal-access-token-create"
                type="button"
                :disabled="personalAccessTokensCreating || !personalAccessTokenCanCreate"
                :aria-busy="personalAccessTokensCreating"
                @click="createPersonalAccessToken"
              >
                <UIcon name="i-lucide-loader-circle" v-if="personalAccessTokensCreating" class="h-4 w-4" />
                {{ t("settings.personalAccessTokens.createAction") }}
              </UButton>
            </div>
          </div>
        </div>
      </AppSettingsBlock>

      <AppSettingsBlock
        v-if="personalAccessTokensReveal"
        compact
        :title="t('settings.personalAccessTokens.revealTitle')"
      >
        <div class="grid gap-3">
          <UAlert color="warning" variant="subtle" :description="t('settings.personalAccessTokens.revealWarning')" />
          <pre
            data-testid="personal-access-token-secret"
            class="overflow-x-auto whitespace-pre-wrap break-all rounded-lg bg-surface-0 dark:bg-surface-950 p-3 font-mono text-xs text-color"
          >{{ personalAccessTokensReveal.access_token }}</pre>
          <div class="flex flex-wrap items-center gap-3">
            <UButton color="neutral" variant="outline" type="button" @click="copyPersonalAccessToken">
              {{ t("settings.personalAccessTokens.copyAction") }}
            </UButton>
            <UButton color="neutral" variant="outline" type="button" @click="dismissPersonalAccessTokenReveal">
              {{ t("common.close") }}
            </UButton>
          </div>
        </div>
      </AppSettingsBlock>

      <AppSettingsBlock compact>
        <UTable
          class="min-w-0 max-w-full"
          :data="tokenRows"
          :columns="columns"
          :loading="personalAccessTokensLoading"
        >
          <template #empty>
            {{ t("settings.personalAccessTokens.empty") }}
          </template>
          <template #status-cell="{ row }"><UBadge :label="row.original.status.label" :color="row.original.status.color" variant="subtle" /></template>
          <template #actions-cell="{ row }">
              <UButton
                type="button"
                size="sm"
                color="error"
                variant="outline"
                :disabled="!!row.original.revoked_at"
                @click="confirmRevokePersonalAccessToken(row.original)"
              >
                {{ t("settings.personalAccessTokens.revokeAction") }}
              </UButton>
          </template>
        </UTable>
        <TablePagination
          :pagination="personalAccessTokensPagination"
          @update:page="emit('page', $event)"
          @update:page-size="emit('page-size', $event)"
        />
      </AppSettingsBlock>
    </div>
  </AppSettingsSection>
</template>
