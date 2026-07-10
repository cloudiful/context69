<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type { DraftRuntimeSettings } from "../../utils/settings";

type QdrantToggleModel = { recreate_on_dimension_mismatch: boolean };
type SchedulerToggleModel = { run_on_start: boolean };

const props = defineProps<{
  qdrantToggleModel: QdrantToggleModel;
  runtimeDraft: DraftRuntimeSettings;
  schedulerToggleModel: SchedulerToggleModel;
}>();

const emit = defineEmits<{
  "update:qdrantToggleModel": [value: QdrantToggleModel];
  "update:schedulerToggleModel": [value: SchedulerToggleModel];
}>();

const { t } = useI18n();

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

function updateEmbeddingApiKeyClear(value: Record<string, boolean>) {
  props.runtimeDraft.embedding.clear_api_key = !!value.clear_api_key;
  if (props.runtimeDraft.embedding.clear_api_key) {
    props.runtimeDraft.embedding.api_key = "";
  }
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.runtime.title')">
    <div class="grid gap-6">
      <section id="settings-embedding" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.embeddingTitle") }}</h3>
        <div class="grid gap-3">
          <div class="settings-compact-grid settings-compact-grid-vlm-main">
            <AppTextField
              float-label
              input-id="runtime-embedding-base-url"
              v-model="runtimeDraft.embedding.base_url"
              :label="t('settings.runtime.embeddingBaseUrl')"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />
            <AppTextField
              float-label
              input-id="runtime-embedding-api-key"
              v-model="runtimeDraft.embedding.api_key"
              :label="t('settings.runtime.embeddingApiKey')"
              type="password"
              autocomplete="new-password"
              placeholder="sk-..."
            />
          </div>
          <div class="settings-compact-grid settings-compact-grid-conversion">
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
          <AppToggleGroup
            v-if="runtimeDraft.embedding.has_api_key"
            :model-value="{ clear_api_key: runtimeDraft.embedding.clear_api_key }"
            columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-single"
            :items="[
              { key: 'clear_api_key', inputId: 'runtime-embedding-clear-api-key', label: t('settings.runtime.clearStoredEmbeddingApiKey'), testId: 'runtime-embedding-clear-api-key' },
            ]"
            @update:model-value="updateEmbeddingApiKeyClear"
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
