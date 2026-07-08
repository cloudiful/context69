<script setup lang="ts">
import Tag from "primevue/tag";

import { useProjectWorkspaceContext } from "../../composables/project-workspace-context";

const state = useProjectWorkspaceContext();
</script>

<template>
  <section class="workspace-block">
    <dl class="workspace-overview-pairs">
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("project.summary.group") }}</dt>
        <dd class="workspace-overview-value">{{ state.groupKey }}</dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("project.summary.project") }}</dt>
        <dd class="workspace-overview-value">{{ state.projectKey }}</dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.projectName") }}</dt>
        <dd class="workspace-overview-value">{{ state.project?.name || "--" }}</dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("project.summary.visibility") }}</dt>
        <dd class="workspace-overview-value">
          <Tag :value="state.project?.visibility || '--'" severity="secondary" />
        </dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.currentRole") }}</dt>
        <dd class="workspace-overview-value">
          <Tag :value="state.project?.current_role || '--'" :severity="state.roleSeverity(state.project?.current_role)" />
        </dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("project.summary.members") }}</dt>
        <dd class="workspace-overview-value">{{ state.members.length }}</dd>
      </div>
    </dl>
  </section>
</template>
