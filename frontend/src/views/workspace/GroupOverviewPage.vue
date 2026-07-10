<script setup lang="ts">
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <div class="workspace-overview-layout">
    <section class="workspace-overview-main">
      <ProjectFilesPanel
        :child-groups="state.childGroups"
        :group-path="state.groupPath"
        @create-child-group="state.openCreateChildGroupDialog"
        @delete-child-group="state.confirmDeleteChildGroup"
        @edit-child-group="state.openEditChildGroupDialog"
        @move-child-group="state.openMoveChildGroupDialog"
        @open-child-group="state.openGroup"
      />
    </section>

    <aside class="workspace-overview-side">
      <section class="workspace-summary-card">
        <p class="section-title">{{ $t("groups.tabs.overview") }}</p>
        <dl class="workspace-summary-list">
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.groupKey") }}</dt>
            <dd class="workspace-summary-value">{{ state.group?.group_key || state.groupKey }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.groupPath") }}</dt>
            <dd class="workspace-summary-value">{{ state.group?.group_path || state.groupPath }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.groupName") }}</dt>
            <dd class="workspace-summary-value">{{ state.group?.name || "--" }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.visibility") }}</dt>
            <dd class="workspace-summary-value">
              <Tag :value="state.group?.visibility || '--'" severity="secondary" />
            </dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.kind") }}</dt>
            <dd class="workspace-summary-value">
              <Tag :value="state.group?.kind || '--'" severity="contrast" />
            </dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.currentRole") }}</dt>
            <dd class="workspace-summary-value">
              <Tag :value="state.group?.current_role || '--'" :severity="state.roleSeverity(state.group?.current_role)" />
            </dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.membersTitle") }}</dt>
            <dd class="workspace-summary-value">{{ state.members.length }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.childrenTitle") }}</dt>
            <dd class="workspace-summary-value">{{ state.childGroups.length }}</dd>
          </div>
        </dl>
      </section>
    </aside>
  </div>
</template>
