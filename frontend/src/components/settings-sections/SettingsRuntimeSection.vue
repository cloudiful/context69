<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Message from "primevue/message";
import Tag from "primevue/tag";

import AppNumberField from "../AppNumberField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type { ProviderAccountResponse } from "../../services/api";
import type { DraftRuntimeSettings, ProviderAccountDraft } from "../../utils/settings";

type ProviderToggleModel = { disabled: boolean };
type QdrantToggleModel = { recreate_on_dimension_mismatch: boolean };
type SchedulerToggleModel = { run_on_start: boolean };

defineProps<{
  deleteProviderAccount: () => void;
  providerAccountOptions: Array<{ label: string; value: string }>;
  providerDraft: ProviderAccountDraft;
  providerKindOptions: Array<{ label: string; value: string }>;
  providerMessage: string;
  providerSaving: boolean;
  providerStatusLabel: string;
  providerToggleModel: ProviderToggleModel;
  qdrantToggleModel: QdrantToggleModel;
  runtimeDraft: DraftRuntimeSettings;
  saving: boolean;
  schedulerToggleModel: SchedulerToggleModel;
  selectedProviderAccount: ProviderAccountResponse | null;
  selectedProviderAccountKey: string;
  startNewProviderAccount: () => void;
  toggleClearProviderApiKey: () => void;
}>();

const emit = defineEmits<{
  "update:selectedProviderAccountKey": [value: string];
  "update:providerToggleModel": [value: ProviderToggleModel];
  "update:qdrantToggleModel": [value: QdrantToggleModel];
  "update:schedulerToggleModel": [value: SchedulerToggleModel];
}>();

const { t } = useI18n();

function updateProviderToggleModel(value: Record<string, boolean>) {
  emit("update:providerToggleModel", {
    disabled: !!value.disabled,
  });
}

function updateQdrantToggleModel(value: Record<string, boolean>) {
  emit("update:qdrantToggleModel", {
    recreate_on_dimension_mismatch: !!value.recreate_on_dimension_mismatch,
  });
}

function updateSchedulerToggleModel(value: Record<string, boolean>) {
  emit("update:schedulerToggleModel", {
    run_on_start: !!value.run_on_start,
  });
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.runtime.title')">
    <div class="grid gap-6">
      <section id="settings-provider-accounts" class="settings-block">
        <div class="settings-block-header">
          <h3 class="settings-block-title">{{ t("settings.runtime.providerAccountsTitle") }}</h3>
          <Button
            class="settings-inline-button"
            type="button"
            severity="secondary"
            variant="outlined"
            @click="startNewProviderAccount"
          >
            {{ t("common.create") }}
          </Button>
        </div>

        <div class="grid gap-3">
          <AppSelectField
            float-label
            input-id="settings-provider-account-select"
            :model-value="selectedProviderAccountKey"
            :label="t('settings.runtime.selectProviderAccount')"
            :options="providerAccountOptions"
            @update:model-value="typeof $event === 'string' && emit('update:selectedProviderAccountKey', $event)"
          />

          <div class="settings-compact-grid settings-compact-grid-models">
            <AppTextField
              float-label
              input-id="provider-account-key"
              v-model="providerDraft.account_key"
              :label="t('settings.runtime.accountKey')"
              :readonly="!!selectedProviderAccount"
            />
            <AppSelectField
              float-label
              input-id="provider-kind"
              v-model="providerDraft.provider_kind"
              :label="t('settings.runtime.providerKind')"
              :options="providerKindOptions"
            />
            <AppTextField
              float-label
              input-id="provider-display-name"
              v-model="providerDraft.display_name"
              :label="t('settings.runtime.displayName')"
            />
          </div>

          <div class="settings-compact-grid settings-compact-grid-vlm-main">
            <AppTextField
              float-label
              input-id="provider-base-url"
              v-model="providerDraft.base_url"
              :label="t('settings.runtime.baseUrl')"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />

            <div class="settings-api-key-shell">
              <AppTextField
                float-label
                input-id="provider-api-key"
                v-model="providerDraft.api_key"
                :label="t('settings.runtime.apiKey')"
                type="password"
                autocomplete="new-password"
                placeholder="sk-..."
              />
              <div class="settings-api-key-side">
                <Tag
                  class="settings-status-tag"
                  :severity="providerDraft.clear_api_key ? 'warn' : (selectedProviderAccount?.has_api_key ? 'success' : 'secondary')"
                  :value="providerStatusLabel"
                />
                <Button
                  id="provider-clear-api-key"
                  class="settings-inline-button"
                  type="button"
                  severity="danger"
                  variant="outlined"
                  :disabled="!selectedProviderAccount?.has_api_key && !providerDraft.clear_api_key"
                  @click="toggleClearProviderApiKey"
                >
                  {{ providerDraft.clear_api_key ? t("settings.runtime.cancelClearApiKey") : t("settings.runtime.clearApiKey") }}
                </Button>
              </div>
            </div>
          </div>

          <div class="settings-toggle-grid-inline settings-toggle-grid-inline-single">
            <AppToggleGroup
              :model-value="providerToggleModel"
              columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-single"
              :items="[
                { key: 'disabled', inputId: 'provider-disabled', label: t('settings.runtime.disableProvider'), testId: 'provider-disabled' },
              ]"
              @update:model-value="updateProviderToggleModel"
            />
          </div>

          <div class="settings-inline-actions">
            <Button
              class="settings-inline-button"
              type="button"
              severity="danger"
              variant="outlined"
              :disabled="saving || providerSaving || !selectedProviderAccount"
              @click="deleteProviderAccount"
            >
              {{ t("common.delete") }}
            </Button>
            <Message v-if="providerMessage" severity="success" :closable="false">
              {{ providerMessage }}
            </Message>
          </div>
        </div>
      </section>

      <section id="settings-embedding" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.embeddingTitle") }}</h3>
        <div class="settings-compact-grid settings-compact-grid-conversion">
          <AppSelectField
            float-label
            input-id="runtime-embedding-provider"
            v-model="runtimeDraft.embedding.provider_account_key"
            :label="t('settings.runtime.embeddingProvider')"
            :options="providerAccountOptions.filter((option) => option.value)"
          />
          <AppTextField
            float-label
            input-id="runtime-embedding-model"
            v-model="runtimeDraft.embedding.model"
            :label="t('settings.runtime.embeddingModel')"
          />
          <AppNumberField
            float-label
            input-id="runtime-embedding-dimensions"
            v-model="runtimeDraft.embedding.dimensions"
            :label="t('settings.runtime.embeddingDimensions')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
            input-id="runtime-embedding-timeout"
            v-model="runtimeDraft.embedding.timeout_secs"
            :label="t('settings.runtime.embeddingTimeout')"
            :min="1"
            :step="1"
          />
        </div>
      </section>

      <section id="settings-qdrant" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.qdrantTitle") }}</h3>
        <div class="settings-compact-grid settings-compact-grid-connection">
          <AppTextField
            float-label
            input-id="runtime-qdrant-url"
            v-model="runtimeDraft.qdrant.url"
            :label="t('settings.runtime.qdrantUrl')"
            type="url"
          />
          <AppTextField
            float-label
            input-id="runtime-qdrant-collection"
            v-model="runtimeDraft.qdrant.collection_name"
            :label="t('settings.runtime.qdrantCollection')"
          />
          <AppToggleGroup
            :model-value="qdrantToggleModel"
            columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-single"
            :items="[
              { key: 'recreate_on_dimension_mismatch', inputId: 'runtime-qdrant-recreate', label: t('settings.runtime.recreateOnDimensionMismatch'), testId: 'runtime-qdrant-recreate' },
            ]"
            @update:model-value="updateQdrantToggleModel"
          />
        </div>
      </section>

      <section id="settings-scheduler" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.schedulerTitle") }}</h3>
        <div class="settings-compact-grid settings-compact-grid-conversion">
          <AppNumberField
            float-label
            input-id="runtime-scheduler-interval"
            v-model="runtimeDraft.scheduler.interval_secs"
            :label="t('settings.runtime.schedulerInterval')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
            input-id="runtime-scheduler-concurrency"
            v-model="runtimeDraft.scheduler.max_concurrency"
            :label="t('settings.runtime.schedulerConcurrency')"
            :min="1"
            :step="1"
          />
          <AppTextField
            float-label
            input-id="runtime-scheduler-job-id"
            v-model="runtimeDraft.scheduler.job_id"
            :label="t('settings.runtime.schedulerJobId')"
          />
          <AppTextField
            float-label
            input-id="runtime-scheduler-valkey-url"
            v-model="runtimeDraft.scheduler.valkey_url"
            :label="t('settings.runtime.schedulerValkeyUrl')"
            placeholder="redis://valkey:6379/0"
          />
          <AppToggleGroup
            :model-value="schedulerToggleModel"
            columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-single"
            :items="[
              { key: 'run_on_start', inputId: 'runtime-scheduler-run-on-start', label: t('settings.runtime.runOnStart'), testId: 'runtime-scheduler-run-on-start' },
            ]"
            @update:model-value="updateSchedulerToggleModel"
          />
        </div>
      </section>

      <section id="settings-chunking" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.chunkingTitle") }}</h3>
        <div class="settings-compact-grid settings-compact-grid-two">
          <AppNumberField
            float-label
            input-id="runtime-chunking-max-chars"
            v-model="runtimeDraft.chunking.max_chars"
            :label="t('settings.runtime.chunkingMaxChars')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
            input-id="runtime-chunking-overlap"
            v-model="runtimeDraft.chunking.overlap_chars"
            :label="t('settings.runtime.chunkingOverlapChars')"
            :min="0"
            :step="1"
          />
        </div>
      </section>

      <section id="settings-file-library" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.fileLibraryTitle") }}</h3>
        <div class="settings-compact-grid settings-compact-grid-models">
          <AppTextField
            float-label
            input-id="runtime-file-library-root"
            v-model="runtimeDraft.file_library.storage_root"
            :label="t('settings.runtime.fileLibraryRoot')"
          />
          <AppNumberField
            float-label
            input-id="runtime-file-library-max-upload"
            v-model="runtimeDraft.file_library.max_upload_size_mb"
            :label="t('settings.runtime.fileLibraryMaxUploadSize')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
            input-id="runtime-file-library-max-request"
            v-model="runtimeDraft.file_library.max_upload_request_size_mb"
            :label="t('settings.runtime.fileLibraryMaxRequestSize')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
            input-id="runtime-file-library-concurrency"
            v-model="runtimeDraft.file_library.ingest_concurrency"
            :label="t('settings.runtime.fileLibraryConcurrency')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
            input-id="runtime-file-library-pages"
            v-model="runtimeDraft.file_library.pdf_pages_per_task"
            :label="t('settings.runtime.fileLibraryPdfPagesPerTask')"
            :min="1"
            :step="1"
          />
        </div>
      </section>
    </div>
  </AppSettingsSection>
</template>
