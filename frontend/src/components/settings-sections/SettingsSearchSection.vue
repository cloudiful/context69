<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type { DraftSearchSettings } from "../../utils/settings";

type RerankToggleModel = { rerank_enabled: boolean };

const props = defineProps<{
  rerankApiKeyDraft: string;
  rerankToggleModel: RerankToggleModel;
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

const boostMargin = computed(() => {
  const vector = Number(props.searchDraft.vector_weight ?? 0);
  const keyword = Number(props.searchDraft.keyword_weight ?? 0);
  return Math.max(0, 1 - vector - keyword).toFixed(2);
});
</script>

<template>
  <AppSettingsSection :legend="t('settings.search.title')">
    <div id="settings-search" class="grid gap-4">
      <AppSettingsBlock compact>
        <div class="grid gap-3">
          <div class="grid max-w-2xl gap-3">
            <AppSelectField
              input-id="search-mode"
              v-model="searchDraft.mode"
              :label="t('settings.search.mode')"
              test-id="search-mode"
              :options="searchModeOptions"
            />

            <AppNumberField
              input-id="search-candidate-limit"
              v-model="searchDraft.candidate_limit"
              :label="t('settings.search.candidateLimit')"
              test-id="search-candidate-limit"
              :min="1"
              :step="1"
            />

            <AppNumberField
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
            columns-class="grid max-w-2xl grid-cols-1 gap-2"
            :items="[
              { key: 'rerank_enabled', inputId: 'search-rerank-enabled', label: t('settings.search.rerankEnabled'), testId: 'search-rerank-enabled' },
            ]"
            @update:model-value="updateRerankToggleModel"
          />

          <div class="grid max-w-2xl gap-3">
            <AppTextField
              input-id="search-rerank-base-url"
              v-model="searchDraft.rerank_base_url"
              :label="t('settings.search.rerankBaseUrl')"
              test-id="search-rerank-base-url"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />

            <AppTextField
              input-id="search-rerank-model"
              v-model="searchDraft.rerank_model"
              :label="t('settings.search.rerankModel')"
              test-id="search-rerank-model"
              placeholder="cohere/rerank-4-fast"
            />

            <AppTextField
              input-id="search-rerank-api-key"
              :model-value="rerankApiKeyDraft"
              :label="t('settings.search.rerankApiKey')"
              test-id="search-rerank-api-key"
              type="password"
              autocomplete="new-password"
              placeholder="sk-or-..."
              @update:model-value="emit('update:rerankApiKeyDraft', $event)"
            />
          </div>

          <div class="grid max-w-2xl gap-3">
            <p class="text-xs text-gray-500 dark:text-gray-400">
              {{ t("settings.search.fusionWeightsHint", { margin: boostMargin }) }}
            </p>

            <AppNumberField
              input-id="search-vector-weight"
              v-model="searchDraft.vector_weight"
              :label="t('settings.search.vectorWeight')"
              test-id="search-vector-weight"
              :min="0"
              :max="1"
              :step="0.05"
            />

            <AppNumberField
              input-id="search-keyword-weight"
              v-model="searchDraft.keyword_weight"
              :label="t('settings.search.keywordWeight')"
              test-id="search-keyword-weight"
              :min="0"
              :max="1"
              :step="0.05"
            />
          </div>
        </div>
      </AppSettingsBlock>
    </div>
  </AppSettingsSection>
</template>
