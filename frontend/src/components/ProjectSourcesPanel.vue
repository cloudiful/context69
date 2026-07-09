<script setup lang="ts">
import ProjectFilesPanel from "./ProjectFilesPanel.vue";

defineProps<{
  groupKey: string;
  projectKey: string;
  canManage?: boolean;
}>();
</script>

<template>
  <ProjectFilesPanel :group-key="groupKey" :project-key="projectKey" />
</template>
