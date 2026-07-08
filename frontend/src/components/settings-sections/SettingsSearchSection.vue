<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type { DraftSearchSettings } from "../../utils/settings";

type RerankToggleModel = { rerank_enabled: boolean };

defineProps<{
  rerankApiKeyDraft: string;
  rerankToggleModel: RerankToggleModel;
  searchHasStoredApiKey: boolean;
  searchDraft: DraftSearchSettings;
  searchModeOptions: Array<{ label: string; value: string }>;
}>();

const emit = defineEmits<{
  "update:rerankApiKeyDraft": [value: string];
  "update:rerankToggleModel": [value: RerankToggleModel];
}>();

const { t } = useI18n();

function updateRerankToggleModel(value: Record<string, boolean>) {
  emit("update:rerankToggleModel", {
    rerank_enabled: !!value.rerank_enabled,
  });
}
</script>

<template>
  <AppSettingsSection :legend="t('settings.search.title')">
    <div id="settings-search" class="grid gap-4">
      <section class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t("settings.search.title") }}</h3>
        <div class="grid gap-3">
          <div class="settings-compact-grid lg:grid-cols-3 lg:items-start xl:grid-cols-[repeat(3,minmax(14rem,18rem))] xl:justify-start">
            <AppSelectField
              float-label
              input-id="search-mode"
              v-model="searchDraft.mode"
              :label="t('settings.search.mode')"
              test-id="search-mode"
              :options="searchModeOptions"
            />

            <AppNumberField
              float-label
              input-id="search-candidate-limit"
              v-model="searchDraft.candidate_limit"
              :label="t('settings.search.candidateLimit')"
              test-id="search-candidate-limit"
              :min="1"
              :step="1"
            />

            <AppNumberField
              float-label
              input-id="search-rerank-timeout"
              v-model="searchDraft.timeout_secs"
              :label="t('settings.search.timeout')"
              test-id="search-rerank-timeout"
              :min="1"
              :step="1"
            />
          </div>

          <AppToggleGroup
            :model-value="rerankToggleModel"
            columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-single"
            :items="[
              { key: 'rerank_enabled', inputId: 'search-rerank-enabled', label: t('settings.search.rerankEnabled'), testId: 'search-rerank-enabled' },
            ]"
            @update:model-value="updateRerankToggleModel"
          />

          <div class="settings-compact-grid lg:grid-cols-3 lg:items-start xl:grid-cols-[minmax(18rem,24rem)_minmax(16rem,20rem)_minmax(18rem,24rem)] xl:justify-start">
            <AppTextField
              float-label
              input-id="search-rerank-base-url"
              v-model="searchDraft.rerank_base_url"
              :label="t('settings.search.rerankBaseUrl')"
              test-id="search-rerank-base-url"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />

            <AppTextField
              float-label
              input-id="search-rerank-model"
              v-model="searchDraft.rerank_model"
              :label="t('settings.search.rerankModel')"
              test-id="search-rerank-model"
              placeholder="cohere/rerank-4-fast"
            />

            <AppTextField
              float-label
              input-id="search-rerank-api-key"
              :model-value="rerankApiKeyDraft"
              :label="t('settings.search.rerankApiKey')"
              :helper="searchHasStoredApiKey
                ? `${t('settings.search.apiKeyStatusStored')} · ${t('settings.hints.apiKey')}`
                : `${t('settings.search.apiKeyStatusMissing')} · ${t('settings.hints.apiKey')}`"
              test-id="search-rerank-api-key"
              type="password"
              autocomplete="new-password"
              placeholder="sk-or-..."
              @update:model-value="emit('update:rerankApiKeyDraft', $event)"
            />
          </div>
        </div>
      </section>
    </div>
  </AppSettingsSection>
</template>
