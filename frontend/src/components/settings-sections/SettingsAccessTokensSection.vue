<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Message from "primevue/message";
import Tag from "primevue/tag";

import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type {
  CreatePersonalAccessTokenResponse,
  PersonalAccessTokenResponse,
} from "../../services/api";
import {
  settingsDangerButtonClass,
  settingsPrimaryButtonClass,
  settingsSecondaryButtonClass,
} from "../../ui/button-classes";

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
  personalAccessTokensCreating: boolean;
  personalAccessTokensError: string;
  personalAccessTokensLoading: boolean;
  personalAccessTokensReveal: CreatePersonalAccessTokenResponse | null;
}>();

const emit = defineEmits<{
  "update:personalAccessTokenScopeToggleModel": [value: Record<string, boolean>];
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
      severity: "danger" as const,
    };
  }
  if (new Date(token.expires_at).getTime() <= Date.now()) {
    return {
      label: t("settings.personalAccessTokens.statusExpired"),
      severity: "warn" as const,
    };
  }
  return {
    label: t("settings.personalAccessTokens.statusActive"),
    severity: "success" as const,
  };
}

function updateScopeToggleModel(value: Record<string, boolean>) {
  emit("update:personalAccessTokenScopeToggleModel", value);
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.personalAccessTokens.title')">
    <div class="grid gap-4">
      <section class="settings-block">
        <div class="grid gap-3">
          <AppToggleGroup
            :model-value="personalAccessTokenScopeToggleModel"
            columns-class="settings-toggle-grid-three"
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
              float-label
              input-id="personal-access-token-name"
              v-model="personalAccessTokenDraft.name"
              :label="t('settings.personalAccessTokens.name')"
              test-id="personal-access-token-name"
            />
            <AppSelectField
              float-label
              input-id="personal-access-token-expiry"
              v-model="personalAccessTokenDraft.expires_in_days"
              :label="t('settings.personalAccessTokens.expiry')"
              :options="personalAccessTokenExpiryOptions"
              test-id="personal-access-token-expiry"
            />
            <div class="flex items-end">
              <Button
                :class="settingsPrimaryButtonClass"
                data-testid="personal-access-token-create"
                type="button"
                :disabled="personalAccessTokensCreating || !personalAccessTokenCanCreate"
                :loading="personalAccessTokensCreating"
                @click="createPersonalAccessToken"
              >
                {{ t("settings.personalAccessTokens.createAction") }}
              </Button>
            </div>
          </div>
        </div>
      </section>

      <Message v-if="personalAccessTokensError" severity="error" :closable="false">
        {{ personalAccessTokensError }}
      </Message>

      <section v-if="personalAccessTokensReveal" class="settings-block">
        <div class="settings-block-header">
          <h3 class="settings-block-title">{{ t("settings.personalAccessTokens.revealTitle") }}</h3>
        </div>
        <div class="grid gap-3">
          <Message severity="warn" :closable="false">
            {{ t("settings.personalAccessTokens.revealWarning") }}
          </Message>
          <pre
            data-testid="personal-access-token-secret"
            class="overflow-x-auto whitespace-pre-wrap break-all rounded-lg bg-(--p-content-background) p-3 font-mono text-xs text-app-text"
          >{{ personalAccessTokensReveal.access_token }}</pre>
          <div class="settings-inline-actions">
            <Button :class="settingsSecondaryButtonClass" type="button" @click="copyPersonalAccessToken">
              {{ t("settings.personalAccessTokens.copyAction") }}
            </Button>
            <Button :class="settingsSecondaryButtonClass" type="button" @click="dismissPersonalAccessTokenReveal">
              {{ t("common.close") }}
            </Button>
          </div>
        </div>
      </section>

      <section class="settings-block">
        <div
          v-if="personalAccessTokensLoading"
          class="rounded-xl border border-app-border/60 bg-app-surface/20 px-4 py-6 text-sm text-app-text-dim"
        >
          {{ t("common.loading") }}
        </div>

        <div
          v-else-if="tokenRows.length === 0"
          class="rounded-xl border border-app-border/60 bg-app-surface/20 px-4 py-6 text-sm text-app-text-dim"
        >
          {{ t("settings.personalAccessTokens.empty") }}
        </div>

        <div v-else class="overflow-x-auto rounded-xl border border-app-border/60 bg-app-surface/20">
          <table class="min-w-[68rem] w-full text-left">
            <thead>
              <tr class="border-b border-app-border/60">
                <th class="whitespace-nowrap px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.name") }}</th>
                <th class="min-w-56 px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.scopes") }}</th>
                <th class="whitespace-nowrap px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.createdAt") }}</th>
                <th class="whitespace-nowrap px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.expiresAt") }}</th>
                <th class="whitespace-nowrap px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.lastUsedAt") }}</th>
                <th class="whitespace-nowrap px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.status") }}</th>
                <th class="whitespace-nowrap px-4 py-3 text-sm font-semibold text-app-text">{{ t("settings.personalAccessTokens.actions") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="token in tokenRows"
                :key="token.token_id"
                class="border-b border-app-border/50 last:border-b-0"
              >
                <td class="whitespace-nowrap px-4 py-3 align-top text-base font-medium text-app-text">{{ token.name }}</td>
                <td class="min-w-64 px-4 py-3 align-top text-sm leading-6 text-app-text-dim">{{ token.scopeSummary }}</td>
                <td class="whitespace-nowrap px-4 py-3 align-top text-sm text-app-text">{{ token.createdLabel }}</td>
                <td class="whitespace-nowrap px-4 py-3 align-top text-sm text-app-text">{{ token.expiresLabel }}</td>
                <td class="whitespace-nowrap px-4 py-3 align-top text-sm text-app-text">{{ token.lastUsedLabel }}</td>
                <td class="whitespace-nowrap px-4 py-3 align-top">
                  <Tag :value="token.status.label" :severity="token.status.severity" />
                </td>
                <td class="whitespace-nowrap px-4 py-3 align-top">
                  <Button
                    :class="settingsDangerButtonClass"
                    type="button"
                    size="small"
                    :disabled="!!token.revoked_at"
                    @click="confirmRevokePersonalAccessToken(token)"
                  >
                    {{ t("settings.personalAccessTokens.revokeAction") }}
                  </Button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>
  </AppSettingsSection>
</template>
