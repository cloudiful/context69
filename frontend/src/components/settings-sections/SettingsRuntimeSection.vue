<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

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
  s3Testing: boolean;
}>();

const emit = defineEmits<{
  "update:qdrantToggleModel": [value: QdrantToggleModel];
  "update:schedulerToggleModel": [value: SchedulerToggleModel];
  "test-s3": [];
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

function updateS3Toggle(value: Record<string, boolean>) {
  props.runtimeDraft.file_library.s3_enabled = !!value.s3_enabled;
}

function updateS3PathStyle(value: Record<string, boolean>) {
  props.runtimeDraft.file_library.s3.path_style = !!value.path_style;
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
              input-id="runtime-embedding-base-url"
              v-model="runtimeDraft.embedding.base_url"
              :label="t('settings.runtime.embeddingBaseUrl')"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />
            <AppTextField
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
              input-id="runtime-embedding-model"
              v-model="runtimeDraft.embedding.model"
              :label="t('settings.runtime.embeddingModel')"
            />
            <AppNumberField
              input-id="runtime-embedding-dimensions"
              v-model="runtimeDraft.embedding.dimensions"
              :label="t('settings.runtime.embeddingDimensions')"
              :min="1"
              :step="1"
            />
            <AppNumberField
              input-id="runtime-embedding-timeout"
              v-model="runtimeDraft.embedding.timeout_secs"
              :label="t('settings.runtime.embeddingTimeout')"
              :min="1"
              :step="1"
            />
          </div>
        </div>
      </section>

      <section id="settings-qdrant" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.runtime.qdrantTitle") }}</h3>
        <div class="settings-compact-grid settings-compact-grid-connection">
          <AppTextField
            input-id="runtime-qdrant-url"
            v-model="runtimeDraft.qdrant.url"
            :label="t('settings.runtime.qdrantUrl')"
            type="url"
          />
          <AppTextField
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
            input-id="runtime-scheduler-interval"
            v-model="runtimeDraft.scheduler.interval_secs"
            :label="t('settings.runtime.schedulerInterval')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="runtime-scheduler-concurrency"
            v-model="runtimeDraft.scheduler.max_concurrency"
            :label="t('settings.runtime.schedulerConcurrency')"
            :min="1"
            :step="1"
          />
          <AppTextField
            input-id="runtime-scheduler-job-id"
            v-model="runtimeDraft.scheduler.job_id"
            :label="t('settings.runtime.schedulerJobId')"
          />
          <AppTextField
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
            input-id="runtime-chunking-max-chars"
            v-model="runtimeDraft.chunking.max_chars"
            :label="t('settings.runtime.chunkingMaxChars')"
            :min="1"
            :step="1"
          />
          <AppNumberField
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
            input-id="runtime-file-library-root"
            v-model="runtimeDraft.file_library.storage_root"
            :label="t('settings.runtime.fileLibraryRoot')"
          />
          <AppNumberField
            input-id="runtime-file-library-max-upload"
            v-model="runtimeDraft.file_library.max_upload_size_mb"
            :label="t('settings.runtime.fileLibraryMaxUploadSize')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="runtime-file-library-max-request"
            v-model="runtimeDraft.file_library.max_upload_request_size_mb"
            :label="t('settings.runtime.fileLibraryMaxRequestSize')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="runtime-file-library-concurrency"
            v-model="runtimeDraft.file_library.ingest_concurrency"
            :label="t('settings.runtime.fileLibraryConcurrency')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="runtime-file-library-pages"
            v-model="runtimeDraft.file_library.pdf_pages_per_task"
            :label="t('settings.runtime.fileLibraryPdfPagesPerTask')"
            :min="1"
            :step="1"
          />
        </div>
        <div class="mt-3 grid gap-3">
          <AppToggleGroup
            helper-inline
            columns-class="grid gap-3"
            :items="[{ inputId: 'runtime-file-library-s3-enabled', key: 's3_enabled', label: t('settings.runtime.s3Enabled') }]"
            :model-value="{ s3_enabled: runtimeDraft.file_library.s3_enabled }"
            @update:model-value="updateS3Toggle"
          />
          <div v-if="runtimeDraft.file_library.s3_enabled" class="grid gap-3">
            <div class="settings-compact-grid settings-compact-grid-two">
              <AppTextField
                input-id="runtime-file-library-s3-endpoint"
                v-model="runtimeDraft.file_library.s3.endpoint"
                :label="t('settings.runtime.s3Endpoint')"
                type="url"
              />
              <AppTextField
                input-id="runtime-file-library-s3-region"
                v-model="runtimeDraft.file_library.s3.region"
                :label="t('settings.runtime.s3Region')"
              />
              <AppTextField
                input-id="runtime-file-library-s3-bucket"
                v-model="runtimeDraft.file_library.s3.bucket"
                :label="t('settings.runtime.s3Bucket')"
              />
              <AppTextField
                input-id="runtime-file-library-s3-prefix"
                v-model="runtimeDraft.file_library.s3.prefix"
                :label="t('settings.runtime.s3Prefix')"
              />
              <AppTextField
                input-id="runtime-file-library-s3-access-key"
                v-model="runtimeDraft.file_library.s3.access_key"
                :label="t('settings.runtime.s3AccessKey')"
              />
              <AppTextField
                input-id="runtime-file-library-s3-secret-key"
                v-model="runtimeDraft.file_library.s3.secret_key"
                :label="t('settings.runtime.s3SecretKey')"
                type="password"
                autocomplete="new-password"
              />
            </div>
            <div class="flex items-center justify-between gap-3">
              <AppToggleGroup
                helper-inline
                columns-class="grid gap-3"
                :items="[{ inputId: 'runtime-file-library-s3-path-style', key: 'path_style', label: t('settings.runtime.s3PathStyle') }]"
                :model-value="{ path_style: runtimeDraft.file_library.s3.path_style }"
                @update:model-value="updateS3PathStyle"
              />
              <Button
                icon="pi pi-bolt"
                :label="t('settings.runtime.s3Test')"
                size="small"
                severity="secondary"
                :loading="s3Testing"
                :disabled="s3Testing"
                @click="emit('test-s3')"
              />
            </div>
          </div>
        </div>
      </section>
    </div>
  </AppSettingsSection>
</template>
