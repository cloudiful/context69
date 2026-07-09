<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsDoclingSection from "../../components/settings-sections/SettingsDoclingSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const doclingApiKeyStatusLabel = computed(() => unref(state.doclingApiKeyStatusLabel));
const doclingHasStoredApiKey = computed(() => unref(state.doclingHasStoredApiKey));
const doclingProviderOptions = computed(() => unref(state.doclingProviderOptions));
</script>

<template>
  <SettingsDoclingSection
    :docling-draft="state.doclingDraft"
    :docling-api-key-status-label="doclingApiKeyStatusLabel"
    :docling-has-stored-api-key="doclingHasStoredApiKey"
    :docling-provider-options="doclingProviderOptions"
    :toggle-clear-docling-api-key="state.toggleClearDoclingApiKey"
  />
</template>
