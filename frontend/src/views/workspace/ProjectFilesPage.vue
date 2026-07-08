<script setup lang="ts">
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import { useProjectWorkspaceContext } from "../../composables/project-workspace-context";

const state = useProjectWorkspaceContext();
</script>

<template>
  <ProjectFilesPanel :group-key="state.groupKey" :project-key="state.projectKey" />
</template>
