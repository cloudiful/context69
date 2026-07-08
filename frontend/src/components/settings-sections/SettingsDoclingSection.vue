<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import type { DraftDoclingSettings } from "../../utils/settings";

defineProps<{
  doclingDraft: DraftDoclingSettings;
  doclingProviderOptions: Array<{ label: string; value: string }>;
}>();

const { t } = useI18n();
</script>

<template>
  <AppSettingsSection :legend="t('settings.docling.title')">
    <div class="grid gap-6">
      <section id="settings-connection" class="grid scroll-mt-16 gap-2.5">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.connectionTitle') }}</h3>
        <div class="grid gap-2 lg:grid-cols-3 lg:items-start xl:grid-cols-[minmax(18rem,24rem)_minmax(10rem,12rem)_minmax(10rem,12rem)] xl:justify-start">
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

      <section id="settings-vlm" class="grid gap-2.5 border-t border-app-border/60 pt-3">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.vlmTitle') }}</h3>
        <div class="grid gap-3">
          <div class="grid gap-2 lg:grid-cols-2 lg:items-start xl:grid-cols-[minmax(16rem,20rem)_minmax(18rem,24rem)] xl:justify-start">
            <AppSelectField
              float-label
              input-id="docling-provider-account"
              v-model="doclingDraft.vlm.provider_account_key"
              :label="t('settings.docling.providerAccount')"
              :options="doclingProviderOptions"
            />
            <AppTextField
              float-label
              input-id="docling-vlm-pipeline-model"
              v-model="doclingDraft.vlm.vlm_pipeline_model"
              :label="t('settings.docling.vlmPipelineModel')"
              placeholder="gemini-3-flash"
            />
          </div>

          <div class="grid gap-2 lg:grid-cols-2 lg:items-start xl:grid-cols-[minmax(18rem,24rem)_minmax(18rem,24rem)] xl:justify-start">
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
