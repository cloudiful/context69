<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import UTooltip from "@nuxt/ui/components/Tooltip.vue";

import AppSelectField from "../AppSelectField.vue";
import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppNumberField from "../AppNumberField.vue";
import type { DraftDoclingSettings, DraftDoclingVlmMode } from "../../utils/settings";

const props = defineProps<{
  doclingDraft: DraftDoclingSettings;
}>();

const { t } = useI18n();

const vlmModeOptions = computed(() => [
  { label: t("settings.docling.vlmModeDisabled"), value: "disabled" },
  { label: t("settings.docling.vlmModePreset"), value: "preset" },
  { label: t("settings.docling.vlmModeCustom"), value: "custom" },
]);

const vlmModeInfo = computed(() => {
  const mode: DraftDoclingVlmMode = props.doclingDraft.vlm_mode;
  switch (mode) {
    case "disabled":
      return t("settings.docling.vlmModeInfoDisabled");
    case "preset":
      return t("settings.docling.vlmModeInfoPreset");
    case "custom":
      return t("settings.docling.vlmModeInfoCustom");
  }
});

const vlmModeInfoLabel = computed(() => t("settings.docling.vlmModeInfoLabel"));
</script>

<template>
  <AppSettingsSection :legend="t('settings.docling.title')">
    <div class="grid gap-6">
      <AppSettingsBlock id="settings-connection" compact :title="t('settings.docling.connectionTitle')">
        <div class="grid max-w-2xl gap-3">
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
          <AppNumberField
            input-id="docling-task-timeout"
            v-model="doclingDraft.connection.task_timeout_secs"
            :label="t('settings.docling.taskTimeout')"
            :min="1"
            :step="1"
          />
        </div>
      </AppSettingsBlock>

      <AppSettingsBlock id="settings-vlm" compact :title="t('settings.docling.vlmTitle')">
        <div class="grid gap-3">
          <div class="grid max-w-2xl gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
            <AppSelectField
              input-id="docling-vlm-mode"
              v-model="doclingDraft.vlm_mode"
              :label="t('settings.docling.vlmModeLabel')"
              :options="vlmModeOptions"
            />
            <UTooltip :text="vlmModeInfo" :ui="{ content: 'max-w-xs text-xs leading-5' }">
              <UButton
                data-testid="docling-vlm-mode-info"
                icon="i-lucide-info"
                color="neutral"
                variant="ghost"
                size="sm"
                :aria-label="vlmModeInfoLabel"
                :title="vlmModeInfoLabel"
              />
            </UTooltip>
          </div>

          <template v-if="doclingDraft.vlm_mode === 'preset'">
            <div class="grid max-w-2xl gap-3">
              <AppTextField
                input-id="docling-picture-description-preset"
                v-model="doclingDraft.vlm.picture_description_preset"
                :label="t('settings.docling.pictureDescriptionPreset')"
                :placeholder="t('settings.docling.pictureDescriptionPresetPlaceholder')"
              />
            </div>
          </template>

          <template v-else-if="doclingDraft.vlm_mode === 'custom'">
            <div class="grid max-w-2xl gap-3">
              <AppTextField
                input-id="docling-openai-base-url"
                v-model="doclingDraft.vlm.openai_base_url"
                :label="t('settings.docling.openAiBaseUrl')"
                type="url"
                placeholder="https://openrouter.ai/api/v1"
              />
            </div>

            <div class="grid max-w-2xl gap-3">
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

            <div class="grid max-w-2xl gap-3">
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
          </template>
        </div>
      </AppSettingsBlock>
    </div>
  </AppSettingsSection>
</template>
