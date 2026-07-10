<script setup lang="ts">
import Button from "primevue/button";
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <div class="workspace-settings-layout">
    <section class="workspace-summary-card">
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
      </dl>
    </section>

    <section class="workspace-summary-card">
      <div class="workspace-action-row">
        <Button v-if="state.canManageGroup" severity="secondary" @click="state.groupDialogVisible = true">
          {{ $t("common.edit") }}
        </Button>
        <Button v-if="state.canManageGroup" severity="secondary" variant="outlined" @click="state.openMoveCurrentGroupDialog">
          {{ $t("common.move") }}
        </Button>
        <Button v-if="state.canOwnGroup" severity="danger" @click="state.confirmDeleteGroup">
          {{ $t("common.delete") }}
        </Button>
      </div>
    </section>
  </div>
</template>
