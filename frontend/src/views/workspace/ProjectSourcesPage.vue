<script setup lang="ts">
import ProjectSourcesPanel from "../../components/ProjectSourcesPanel.vue";
import { useProjectWorkspaceContext } from "../../composables/project-workspace-context";

const state = useProjectWorkspaceContext();
</script>

<template>
  <ProjectSourcesPanel
    :group-key="state.groupKey"
    :project-key="state.projectKey"
    :can-manage="state.canManageProject"
  />
</template>
