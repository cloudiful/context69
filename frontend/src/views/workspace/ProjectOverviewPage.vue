<script setup lang="ts">
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import Tag from "primevue/tag";

import { useProjectWorkspaceContext } from "../../composables/project-workspace-context";

const state = useProjectWorkspaceContext();
</script>

<template>
  <div class="workspace-overview-layout">
    <section class="workspace-overview-main">
      <ProjectFilesPanel :group-key="state.groupKey" :project-key="state.projectKey" />
    </section>

    <aside class="workspace-overview-side">
      <section class="workspace-summary-card">
        <p class="section-title">{{ $t("project.tabs.overview") }}</p>
        <dl class="workspace-summary-list">
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("project.summary.group") }}</dt>
            <dd class="workspace-summary-value">{{ state.groupKey }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("project.summary.project") }}</dt>
            <dd class="workspace-summary-value">{{ state.projectKey }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.projectName") }}</dt>
            <dd class="workspace-summary-value">{{ state.project?.name || "--" }}</dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("project.summary.visibility") }}</dt>
            <dd class="workspace-summary-value">
              <Tag :value="state.project?.visibility || '--'" severity="secondary" />
            </dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("groups.currentRole") }}</dt>
            <dd class="workspace-summary-value">
              <Tag :value="state.project?.current_role || '--'" :severity="state.roleSeverity(state.project?.current_role)" />
            </dd>
          </div>
          <div class="workspace-summary-row">
            <dt class="workspace-summary-label">{{ $t("project.summary.members") }}</dt>
            <dd class="workspace-summary-value">{{ state.members.length }}</dd>
          </div>
        </dl>
      </section>
    </aside>
  </div>
</template>
