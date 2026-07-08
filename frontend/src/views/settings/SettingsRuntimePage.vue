<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsRuntimeSection from "../../components/settings-sections/SettingsRuntimeSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const providerAccountOptions = computed(() => unref(state.providerAccountOptions));
const providerKindOptions = computed(() => unref(state.providerKindOptions));
const providerMessage = computed(() => unref(state.providerMessage));
const providerSaving = computed(() => unref(state.providerSaving));
const providerStatusLabel = computed(() => unref(state.providerStatusLabel));
const providerToggleModel = computed(() => ({
  disabled: unref(state.providerToggleModel).disabled,
}));
const qdrantToggleModel = computed(() => ({
  recreate_on_dimension_mismatch: unref(state.qdrantToggleModel).recreate_on_dimension_mismatch,
}));
const saving = computed(() => unref(state.saving));
const schedulerToggleModel = computed(() => ({
  run_on_start: unref(state.schedulerToggleModel).run_on_start,
}));
const selectedProviderAccount = computed(() => unref(state.selectedProviderAccount));
const selectedProviderAccountKey = computed(() => unref(state.selectedProviderAccountKey));
</script>

<template>
    <SettingsRuntimeSection
      :delete-provider-account="state.deleteProviderAccount"
    :provider-account-options="providerAccountOptions"
    :provider-draft="state.providerDraft"
    :provider-kind-options="providerKindOptions"
    :provider-message="providerMessage"
    :provider-saving="providerSaving"
    :provider-status-label="providerStatusLabel"
    :provider-toggle-model="providerToggleModel"
    :qdrant-toggle-model="qdrantToggleModel"
    :runtime-draft="state.runtimeDraft"
    :saving="saving"
    :scheduler-toggle-model="schedulerToggleModel"
    :selected-provider-account="selectedProviderAccount"
    :selected-provider-account-key="selectedProviderAccountKey"
    :start-new-provider-account="state.startNewProviderAccount"
    :toggle-clear-provider-api-key="state.toggleClearProviderApiKey"
    @update:provider-toggle-model="state.providerToggleModel.value = $event"
    @update:qdrant-toggle-model="state.qdrantToggleModel.value = $event"
    @update:scheduler-toggle-model="state.schedulerToggleModel.value = $event"
    @update:selected-provider-account-key="state.selectedProviderAccountKey.value = $event"
  />
</template>
