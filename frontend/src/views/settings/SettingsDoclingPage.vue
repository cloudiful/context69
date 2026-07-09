<script setup lang="ts">
import { unref } from "vue";

import SettingsDoclingSection from "../../components/settings-sections/SettingsDoclingSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();
</script>

<template>
  <SettingsDoclingSection
    :docling-draft="state.doclingDraft"
    :docling-provider-options="unref(state.doclingProviderOptions)"
  />
</template>
