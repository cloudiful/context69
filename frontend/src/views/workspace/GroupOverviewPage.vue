<script setup lang="ts">
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";
import { toolPrimaryButtonClass } from "../../ui/button-classes";

const state = useGroupWorkspaceContext();

function projectRowClass() {
  return "cursor-pointer";
}
</script>

<template>
  <div class="workspace-overview-layout">
    <section class="workspace-overview-main">
      <ProjectFilesPanel :group-path="state.groupPath" />

      <DataTable
        class="app-data-table"
        :value="state.childGroups"
        data-key="group_id"
        row-hover
        scrollable
        table-style="min-width: 100%"
        :row-class="projectRowClass"
        @row-click="state.openGroup($event.data)"
      >
        <template #empty>
          {{ $t("groups.emptyChildren") }}
        </template>

        <Column>
          <template #header>
            <div class="flex w-full items-center justify-between gap-3">
              <span class="text-sm font-semibold text-app-text">
                {{ $t("groups.childrenTitle") }}
              </span>
              <Button :class="toolPrimaryButtonClass" size="small" @click="state.openCreateChildGroupDialog">
                {{ $t("groups.createChild") }}
              </Button>
            </div>
          </template>
          <template #body="{ data }">
            <div class="flex min-w-0 items-center gap-2">
              <span class="truncate text-sm font-semibold text-app-text">
                {{ data.name }}
              </span>
              <span class="truncate text-sm text-app-text-dim">
                {{ data.group_path || data.group_key }}
              </span>
            </div>
          </template>
        </Column>
        <Column field="visibility" :header="$t('groups.visibility')">
          <template #body="{ data }">
            <Tag :value="data.visibility" severity="secondary" />
          </template>
        </Column>
        <Column :header="$t('common.edit')">
          <template #body="{ data }">
            <div class="flex gap-2">
              <Button severity="secondary" variant="outlined" size="small" @click.stop="state.openGroup(data)">
                {{ $t("common.open") }}
              </Button>
              <Button severity="secondary" variant="outlined" size="small" @click.stop="state.openEditChildGroupDialog(data)">
                {{ $t("common.edit") }}
              </Button>
              <Button severity="secondary" variant="outlined" size="small" @click.stop="state.openMoveChildGroupDialog(data)">
                {{ $t("common.move") }}
              </Button>
              <Button severity="danger" variant="outlined" size="small" @click.stop="state.confirmDeleteChildGroup(data)">
                {{ $t("common.delete") }}
              </Button>
            </div>
          </template>
        </Column>
      </DataTable>
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
