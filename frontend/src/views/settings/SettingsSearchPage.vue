<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsSearchSection from "../../components/settings-sections/SettingsSearchSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const rerankApiKeyDraft = computed(() => unref(state.rerankApiKeyDraft));
const rerankToggleModel = computed(() => ({
  rerank_enabled: unref(state.rerankToggleModel).rerank_enabled,
}));
const searchHasStoredApiKey = computed(() => unref(state.searchHasStoredApiKey));
const searchModeOptions = computed(() => unref(state.searchModeOptions));
</script>

<template>
  <SettingsSearchSection
    :rerank-api-key-draft="rerankApiKeyDraft"
    :rerank-toggle-model="rerankToggleModel"
    :search-has-stored-api-key="searchHasStoredApiKey"
    :search-draft="state.searchDraft"
    :search-mode-options="searchModeOptions"
    @update:rerank-api-key-draft="state.rerankApiKeyDraft.value = $event"
    @update:rerank-toggle-model="state.rerankToggleModel.value = $event"
  />
</template>
