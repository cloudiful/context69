<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import ProgressSpinner from "primevue/progressspinner";

import AppNumberField from "../AppNumberField.vue";
import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type { DraftRuntimeSettings } from "../../utils/settings";

const props = defineProps<{
  runtimeDraft: DraftRuntimeSettings;
  s3Testing: boolean;
}>();
const emit = defineEmits<{ "test-s3": [] }>();
const { t } = useI18n();

function updateLibraryToggles(value: Record<string, boolean>) {
  props.runtimeDraft.file_library.trusted_proxy_enabled = !!value.trusted_proxy_enabled;
  props.runtimeDraft.file_library.s3_enabled = !!value.s3_enabled;
}

function updateS3PathStyle(value: Record<string, boolean>) {
  props.runtimeDraft.file_library.s3.path_style = !!value.path_style;
}
</script>

<template>
  <AppSettingsBlock id="settings-file-library" compact :title="t('settings.runtime.fileLibraryTitle')">
    <div class="grid max-w-2xl gap-3">
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
        columns-class="grid max-w-2xl gap-3"
        :items="[
          {
            inputId: 'runtime-file-library-trusted-proxy',
            key: 'trusted_proxy_enabled',
            label: t('settings.runtime.trustedProxyEnabled'),
            helper: t('settings.runtime.trustedProxyHelper'),
            testId: 'runtime-file-library-trusted-proxy',
          },
          {
            inputId: 'runtime-file-library-s3-enabled',
            key: 's3_enabled',
            label: t('settings.runtime.s3Enabled'),
          },
        ]"
        :model-value="{
          trusted_proxy_enabled: runtimeDraft.file_library.trusted_proxy_enabled,
          s3_enabled: runtimeDraft.file_library.s3_enabled,
        }"
        @update:model-value="updateLibraryToggles"
      />
      <div v-if="runtimeDraft.file_library.s3_enabled" class="grid gap-3">
        <div class="grid max-w-2xl gap-3">
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
        <div class="grid max-w-2xl gap-3">
          <AppToggleGroup
            helper-inline
            columns-class="grid max-w-2xl gap-3"
            :items="[{ inputId: 'runtime-file-library-s3-path-style', key: 'path_style', label: t('settings.runtime.s3PathStyle') }]"
            :model-value="{ path_style: runtimeDraft.file_library.s3.path_style }"
            @update:model-value="updateS3PathStyle"
          />
          <Button
            size="small"
            severity="secondary"
            :disabled="s3Testing"
            :aria-busy="s3Testing"
            @click="emit('test-s3')"
          >
            <ProgressSpinner v-if="s3Testing" class="h-4 w-4" :stroke-width="6" />
            <i v-else class="pi pi-bolt" aria-hidden="true" />
            <span>{{ t("settings.runtime.s3Test") }}</span>
          </Button>
        </div>
      </div>
    </div>
  </AppSettingsBlock>
</template>
