<script setup lang="ts">
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <section class="workspace-block">
    <dl class="workspace-overview-pairs">
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.groupKey") }}</dt>
        <dd class="workspace-overview-value">{{ state.group?.group_key || state.groupKey }}</dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.groupName") }}</dt>
        <dd class="workspace-overview-value">{{ state.group?.name || "--" }}</dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.visibility") }}</dt>
        <dd class="workspace-overview-value">
          <Tag :value="state.group?.visibility || '--'" severity="secondary" />
        </dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.kind") }}</dt>
        <dd class="workspace-overview-value">
          <Tag :value="state.group?.kind || '--'" severity="contrast" />
        </dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.currentRole") }}</dt>
        <dd class="workspace-overview-value">
          <Tag :value="state.group?.current_role || '--'" :severity="state.roleSeverity(state.group?.current_role)" />
        </dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.projectsTitle") }}</dt>
        <dd class="workspace-overview-value">{{ state.projects.length }}</dd>
      </div>
      <div class="workspace-overview-pair">
        <dt class="workspace-overview-key">{{ $t("groups.membersTitle") }}</dt>
        <dd class="workspace-overview-value">{{ state.members.length }}</dd>
      </div>
    </dl>
  </section>
</template>
