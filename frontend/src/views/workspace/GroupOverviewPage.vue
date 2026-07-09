<script setup lang="ts">
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
      <DataTable
        class="app-data-table"
        :value="state.projects"
        data-key="project_id"
        row-hover
        scrollable
        table-style="min-width: 100%"
        :row-class="projectRowClass"
        @row-click="state.openProject($event.data)"
      >
        <template #empty>
          {{ $t("groups.emptyProjects") }}
        </template>

        <Column>
          <template #header>
            <div class="flex w-full items-center justify-between gap-3">
              <span class="text-sm font-semibold text-app-text">
                {{ $t("groups.projectName") }}
              </span>
              <Button :class="toolPrimaryButtonClass" size="small" @click="state.openCreateProjectDialog">
                {{ $t("groups.createProject") }}
              </Button>
            </div>
          </template>
          <template #body="{ data }">
            <div class="flex min-w-0 items-center gap-2">
              <span class="truncate text-sm font-semibold text-app-text">
                {{ data.name }}
              </span>
              <span class="truncate text-sm text-app-text-dim">
                {{ data.project_key }}
              </span>
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
        </dl>
      </section>
    </aside>
  </div>
</template>
