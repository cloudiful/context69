<script setup lang="ts">
import { computed, unref } from "vue";

import SettingsDoclingSection from "../../components/settings-sections/SettingsDoclingSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
const doclingProviderOptions = computed(() => unref(state.doclingProviderOptions));
</script>

<template>
  <SettingsDoclingSection
    :docling-draft="state.doclingDraft"
    :docling-provider-options="doclingProviderOptions"
  />
</template>
