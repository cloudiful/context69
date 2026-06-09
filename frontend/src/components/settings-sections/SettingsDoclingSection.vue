<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import AppNumberPresetField from "../AppNumberPresetField.vue";
import AppSelectField from "../AppSelectField.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppToggleGroup from "../AppToggleGroup.vue";
import type { DraftDoclingSettings } from "../../utils/settings";

type OcrToggleModel = { do_ocr: boolean; force_ocr: boolean };
type EnrichmentToggleModel = {
  do_code_enrichment: boolean;
  do_formula_enrichment: boolean;
  do_picture_description: boolean;
};

defineProps<{
  doclingDraft: DraftDoclingSettings;
  doclingProviderOptions: Array<{ label: string; value: string }>;
  enrichmentToggleModel: EnrichmentToggleModel;
  imageExportModeOptions: Array<{ label: string; value: string }>;
  ocrEngineOptions: Array<{ label: string; value: string }>;
  ocrLangText: string;
  ocrToggleModel: OcrToggleModel;
  pdfBackendOptions: Array<{ label: string; value: string }>;
  pollPresetOptions: Array<{ label: string; value: number }>;
  selectedPollPreset: number | null;
  selectedTimeoutPreset: number | null;
  timeoutPresetOptions: Array<{ label: string; value: number }>;
}>();

const emit = defineEmits<{
  "update:enrichmentToggleModel": [value: EnrichmentToggleModel];
  "update:ocrToggleModel": [value: OcrToggleModel];
  "update:ocrLangText": [value: string];
  "update:selectedPollPreset": [value: number | null];
  "update:selectedTimeoutPreset": [value: number | null];
}>();

const { t } = useI18n();

function updateOcrToggleModel(value: Record<string, boolean>) {
  emit("update:ocrToggleModel", {
    do_ocr: !!value.do_ocr,
    force_ocr: !!value.force_ocr,
  });
}

function updateEnrichmentToggleModel(value: Record<string, boolean>) {
  emit("update:enrichmentToggleModel", {
    do_code_enrichment: !!value.do_code_enrichment,
    do_formula_enrichment: !!value.do_formula_enrichment,
    do_picture_description: !!value.do_picture_description,
  });
}
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
          <AppNumberPresetField
            input-id="docling-timeout"
            :model-value="doclingDraft.connection.timeout_secs"
            :preset-value="selectedTimeoutPreset"
            :label="t('settings.docling.timeout')"
            :preset-options="timeoutPresetOptions"
            :min="1"
            :step="1"
            @update:model-value="doclingDraft.connection.timeout_secs = $event ?? doclingDraft.connection.timeout_secs"
            @update:preset-value="emit('update:selectedTimeoutPreset', $event)"
          />
          <AppNumberPresetField
            input-id="docling-poll-interval"
            :model-value="doclingDraft.connection.poll_interval_secs"
            :preset-value="selectedPollPreset"
            :label="t('settings.docling.pollInterval')"
            :preset-options="pollPresetOptions"
            :min="1"
            :step="1"
            @update:model-value="doclingDraft.connection.poll_interval_secs = $event ?? doclingDraft.connection.poll_interval_secs"
            @update:preset-value="emit('update:selectedPollPreset', $event)"
          />
        </div>
      </section>

      <section id="settings-ocr" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.ocrTitle') }}</h3>
        <div class="settings-compact-grid settings-compact-grid-ocr">
          <AppToggleGroup
            :model-value="ocrToggleModel"
            columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-two"
            :items="[
              { key: 'do_ocr', inputId: 'docling-do-ocr', label: t('settings.docling.doOcr'), testId: 'docling-do-ocr' },
              { key: 'force_ocr', inputId: 'docling-force-ocr', label: t('settings.docling.forceOcr'), testId: 'docling-force-ocr' },
            ]"
            @update:model-value="updateOcrToggleModel"
          />
          <AppSelectField
            input-id="docling-ocr-engine"
            v-model="doclingDraft.ocr.ocr_engine"
            :label="t('settings.docling.ocrEngine')"
            :options="ocrEngineOptions"
          />
          <AppTextField
            input-id="docling-ocr-lang"
            :model-value="ocrLangText"
            :label="t('settings.docling.ocrLang')"
            placeholder="en, zh"
            @update:model-value="emit('update:ocrLangText', $event)"
          />
        </div>
      </section>

      <section id="settings-conversion" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.conversionTitle') }}</h3>
        <div class="settings-compact-grid settings-compact-grid-conversion">
          <AppSelectField
            input-id="docling-pdf-backend"
            v-model="doclingDraft.conversion.pdf_backend"
            :label="t('settings.docling.pdfBackend')"
            :options="pdfBackendOptions"
          />
          <AppNumberField
            input-id="docling-images-scale"
            v-model="doclingDraft.conversion.images_scale"
            :label="t('settings.docling.imagesScale')"
            :min="0.1"
            :step="0.1"
          />
          <AppSelectField
            input-id="docling-image-export-mode"
            v-model="doclingDraft.conversion.image_export_mode"
            :label="t('settings.docling.imageExportMode')"
            :options="imageExportModeOptions"
          />
        </div>
      </section>

      <section id="settings-enrichment" class="settings-block">
        <h3 class="text-sm font-semibold text-app-text">{{ t('settings.docling.enrichmentTitle') }}</h3>
        <div class="grid gap-3">
          <AppToggleGroup
            :model-value="enrichmentToggleModel"
            columns-class="settings-toggle-grid-inline settings-toggle-grid-inline-fit"
            :items="[
              { key: 'do_code_enrichment', inputId: 'docling-code-enrichment', label: t('settings.docling.doCodeEnrichment'), testId: 'docling-code-enrichment' },
              { key: 'do_formula_enrichment', inputId: 'docling-formula-enrichment', label: t('settings.docling.doFormulaEnrichment'), testId: 'docling-formula-enrichment' },
              { key: 'do_picture_description', inputId: 'docling-picture-description', label: t('settings.docling.doPictureDescription'), testId: 'docling-picture-description' },
            ]"
            @update:model-value="updateEnrichmentToggleModel"
          />

          <div class="settings-compact-grid settings-compact-grid-vlm-main">
            <AppSelectField
              input-id="docling-provider-account"
              v-model="doclingDraft.vlm.provider_account_key"
              :label="t('settings.docling.providerAccount')"
              :options="doclingProviderOptions"
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
