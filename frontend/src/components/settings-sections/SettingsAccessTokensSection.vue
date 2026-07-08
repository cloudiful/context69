<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
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
  personalAccessTokenValidationError: string;
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
          <AppTextField
            float-label
            input-id="personal-access-token-name"
            v-model="personalAccessTokenDraft.name"
            :label="t('settings.personalAccessTokens.name')"
            test-id="personal-access-token-name"
          />

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

          <p
            v-if="personalAccessTokenValidationError"
            class="settings-validation-text"
            data-testid="personal-access-token-validation"
          >
            {{ personalAccessTokenValidationError }}
          </p>

          <div class="settings-compact-grid settings-compact-grid-two">
            <AppSelectField
              float-label
              input-id="personal-access-token-expiry"
              v-model="personalAccessTokenDraft.expires_in_days"
              :label="t('settings.personalAccessTokens.expiry')"
              :options="personalAccessTokenExpiryOptions"
              test-id="personal-access-token-expiry"
            />

            <div class="settings-inline-actions items-end">
              <Button
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
            <Button type="button" severity="secondary" variant="outlined" @click="copyPersonalAccessToken">
              {{ t("settings.personalAccessTokens.copyAction") }}
            </Button>
            <Button type="button" severity="secondary" variant="text" @click="dismissPersonalAccessTokenReveal">
              {{ t("common.close") }}
            </Button>
          </div>
        </div>
      </section>

      <section class="settings-block">
        <div class="settings-block-header">
          <h3 class="settings-block-title">{{ t("settings.personalAccessTokens.listTitle") }}</h3>
        </div>

        <DataTable
          :value="tokenRows"
          data-key="token_id"
          size="small"
          class="text-xs"
          :loading="personalAccessTokensLoading"
          responsive-layout="scroll"
        >
          <Column field="name" :header="t('settings.personalAccessTokens.name')" />
          <Column field="scopeSummary" :header="t('settings.personalAccessTokens.scopes')" />
          <Column field="createdLabel" :header="t('settings.personalAccessTokens.createdAt')" />
          <Column field="expiresLabel" :header="t('settings.personalAccessTokens.expiresAt')" />
          <Column field="lastUsedLabel" :header="t('settings.personalAccessTokens.lastUsedAt')" />
          <Column :header="t('settings.personalAccessTokens.status')">
            <template #body="{ data }">
              <Tag :value="data.status.label" :severity="data.status.severity" />
            </template>
          </Column>
          <Column :header="t('settings.personalAccessTokens.actions')">
            <template #body="{ data }">
              <div class="settings-inline-actions">
                <Button
                  type="button"
                  severity="danger"
                  variant="outlined"
                  size="small"
                  :disabled="!!data.revoked_at"
                  @click="confirmRevokePersonalAccessToken(data)"
                >
                  {{ t("settings.personalAccessTokens.revokeAction") }}
                </Button>
              </div>
            </template>
          </Column>
        </DataTable>
      </section>
    </div>
  </AppSettingsSection>
</template>
