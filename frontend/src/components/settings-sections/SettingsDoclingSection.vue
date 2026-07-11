<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import type { DraftDoclingSettings } from "../../utils/settings";

defineProps<{
  doclingDraft: DraftDoclingSettings;
}>();

const { t } = useI18n();

</script>

<template>
  <AppSettingsSection :legend="t('settings.docling.title')">
    <div class="grid gap-6">
      <section id="settings-connection" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.connectionTitle') }}</h3>
        <div class="settings-compact-grid settings-compact-grid-connection">
          <AppTextField
            input-id="docling-base-url"
            v-model="doclingDraft.connection.base_url"
            :label="t('settings.docling.baseUrl')"
            type="url"
            placeholder="http://127.0.0.1:5001"
          />
          <AppNumberField
            input-id="docling-timeout"
            v-model="doclingDraft.connection.timeout_secs"
            :label="t('settings.docling.timeout')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="docling-poll-interval"
            v-model="doclingDraft.connection.poll_interval_secs"
            :label="t('settings.docling.pollInterval')"
            :min="1"
            :step="1"
          />
        </div>
      </section>

      <section id="settings-vlm" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.vlmTitle') }}</h3>
        <div class="grid gap-3">
          <div class="settings-compact-grid settings-compact-grid-vlm-main">
            <AppTextField
              input-id="docling-openai-base-url"
              v-model="doclingDraft.vlm.openai_base_url"
              :label="t('settings.docling.openAiBaseUrl')"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />
          </div>

          <div class="settings-compact-grid settings-compact-grid-vlm-main">
            <AppTextField
              input-id="docling-api-key"
              v-model="doclingDraft.vlm.api_key"
              :label="t('settings.docling.apiKey')"
              type="password"
              autocomplete="new-password"
              placeholder="sk-..."
            />
            <AppTextField
              input-id="docling-vlm-pipeline-model"
              v-model="doclingDraft.vlm.vlm_pipeline_model"
              :label="t('settings.docling.vlmPipelineModel')"
              placeholder="gemini-3-flash"
            />
          </div>

          <div class="settings-compact-grid settings-compact-grid-models">
            <AppTextField
              input-id="docling-picture-description-model"
              v-model="doclingDraft.vlm.picture_description_model"
              :label="t('settings.docling.pictureDescriptionModel')"
              placeholder="gpt-4o-mini"
            />
            <AppTextField
              input-id="docling-code-formula-model"
              v-model="doclingDraft.vlm.code_formula_model"
              :label="t('settings.docling.codeFormulaModel')"
              placeholder="gpt-4o-mini"
            />
          </div>
        </div>
      </section>
    </div>
  </AppSettingsSection>
</template>
