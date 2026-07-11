<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsRuntimeSection from "../../components/settings-sections/SettingsRuntimeSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const qdrantToggleModel = computed(() => ({
  recreate_on_dimension_mismatch: unref(state.qdrantToggleModel).recreate_on_dimension_mismatch,
}));
const schedulerToggleModel = computed(() => ({
  run_on_start: unref(state.schedulerToggleModel).run_on_start,
}));
</script>

<template>
  <SettingsRuntimeSection
    :qdrant-toggle-model="qdrantToggleModel"
    :runtime-draft="state.runtimeDraft"
    :scheduler-toggle-model="schedulerToggleModel"
    :s3-testing="state.s3Testing.value"
    :valkey-testing="state.valkeyTesting.value"
    @update:qdrant-toggle-model="state.qdrantToggleModel.value = $event"
    @update:scheduler-toggle-model="state.schedulerToggleModel.value = $event"
    @test-s3="state.testS3Connection"
    @test-valkey="state.testValkeyConnection"
  />
</template>
