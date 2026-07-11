<script setup lang="ts">
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <ProjectFilesPanel
    :child-groups="state.childGroups"
    :group-path="state.groupPath"
    @create-child-group="state.openCreateChildGroupDialog"
    @delete-child-group="state.confirmDeleteChildGroup"
    @edit-child-group="state.openEditChildGroupDialog"
    @move-child-group="state.openMoveChildGroupDialog"
    @open-child-group="state.openGroup"
  />
</template>
