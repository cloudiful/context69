<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Tag from "primevue/tag";

import AppNumberField from "../AppNumberField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import {
  settingsDangerButtonClass,
  settingsSecondaryButtonClass,
} from "../../ui/button-classes";
import type { DraftDoclingSettings } from "../../utils/settings";

defineProps<{
  doclingApiKeyStatusLabel: string;
  doclingDraft: DraftDoclingSettings;
  doclingHasStoredApiKey: boolean;
  doclingProviderOptions: Array<{ label: string; value: string }>;
  toggleClearDoclingApiKey: () => void;
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
            float-label
            input-id="docling-base-url"
            v-model="doclingDraft.connection.base_url"
            :label="t('settings.docling.baseUrl')"
            type="url"
            placeholder="http://127.0.0.1:5001"
          />
          <AppNumberField
            float-label
            input-id="docling-timeout"
            v-model="doclingDraft.connection.timeout_secs"
            :label="t('settings.docling.timeout')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            float-label
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
            <AppSelectField
              float-label
              input-id="docling-provider-account"
              v-model="doclingDraft.vlm.provider_account_key"
              :label="t('settings.docling.providerAccount')"
              :options="doclingProviderOptions"
            />
            <AppTextField
              float-label
              input-id="docling-openai-base-url"
              v-model="doclingDraft.vlm.openai_base_url"
              :label="t('settings.docling.openAiBaseUrl')"
              type="url"
              placeholder="https://openrouter.ai/api/v1"
            />
          </div>

          <div class="settings-compact-grid settings-compact-grid-vlm-main">
            <div class="settings-api-key-shell">
              <AppTextField
                float-label
                input-id="docling-api-key"
                v-model="doclingDraft.vlm.api_key"
                :label="t('settings.docling.apiKey')"
                type="password"
                autocomplete="new-password"
                placeholder="sk-..."
              />
              <div class="settings-api-key-side">
                <Tag
                  class="settings-status-tag"
                  :severity="doclingDraft.vlm.clear_api_key ? 'warn' : (doclingHasStoredApiKey ? 'success' : 'secondary')"
                  :value="doclingApiKeyStatusLabel"
                />
                <Button
                  id="docling-clear-api-key"
                  :class="doclingDraft.vlm.clear_api_key ? settingsSecondaryButtonClass : settingsDangerButtonClass"
                  type="button"
                  :disabled="!doclingHasStoredApiKey && !doclingDraft.vlm.clear_api_key"
                  @click="toggleClearDoclingApiKey"
                >
                  {{ doclingDraft.vlm.clear_api_key ? t("settings.docling.cancelClearApiKey") : t("settings.docling.clearApiKey") }}
                </Button>
              </div>
            </div>
            <AppTextField
              float-label
              input-id="docling-vlm-pipeline-model"
              v-model="doclingDraft.vlm.vlm_pipeline_model"
              :label="t('settings.docling.vlmPipelineModel')"
              placeholder="gemini-3-flash"
            />
          </div>

          <div class="settings-compact-grid settings-compact-grid-models">
            <AppTextField
              float-label
              input-id="docling-picture-description-model"
              v-model="doclingDraft.vlm.picture_description_model"
              :label="t('settings.docling.pictureDescriptionModel')"
              placeholder="gpt-4o-mini"
            />
            <AppTextField
              float-label
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
