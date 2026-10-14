<script setup lang="ts">
import { onMounted } from "vue";
import SettingsDoclingSection from "../../components/settings-sections/SettingsDoclingSection.vue";
import { useSettingsPageContext } from "../../composables/settings-page-context";

const state = useSettingsPageContext();

onMounted(() => {
  if (!state.doclingRemoteLoadedOnce.value && !state.doclingRemoteLoading.value) {
    void state.loadDoclingRemoteTasks();
  }
});
</script>

<template>
  <SettingsDoclingSection
    :docling-draft="state.doclingDraft"
    :docling-testing="state.doclingTesting.value"
    :remote-items="state.doclingRemoteItems.value"
    :cleanup-items="state.doclingCleanupItems.value"
    :cleaning="state.doclingCleaning.value"
    @clean-stopped="state.cleanStoppedDoclingTasks"
    :remote-loading="state.doclingRemoteLoading.value"
    :remote-error="state.doclingRemoteError.value"
    :remote-error-status="state.doclingRemoteErrorStatus.value"
    :remote-has-more="state.doclingRemoteHasMore.value"
    :remote-offset="state.doclingRemoteOffset.value"
    :remote-loaded-once="state.doclingRemoteLoadedOnce.value"
    @test-docling="state.testDoclingConnection"
    @refresh-remote="state.refreshDoclingRemoteTasks"
    @prev-remote="state.prevDoclingRemotePage"
    @next-remote="state.nextDoclingRemotePage"
  />
</template>
