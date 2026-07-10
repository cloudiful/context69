<script setup lang="ts">
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <div class="grid gap-4 xl:grid-cols-[minmax(0,1.7fr)_18rem] xl:items-start">
    <section class="grid min-w-0 gap-3">
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

    <aside class="grid gap-3">
      <section class="grid gap-3 rounded-[1rem] border border-app-border/65 bg-app-surface-muted/18 p-4">
        <p class="section-title">{{ $t("groups.tabs.overview") }}</p>
        <dl class="grid gap-2">
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.groupKey") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">{{ state.group?.group_key || state.groupKey }}</dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.groupPath") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">{{ state.group?.group_path || state.groupPath }}</dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.groupName") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">{{ state.group?.name || "--" }}</dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.visibility") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">
              <Tag class="justify-self-end" :value="state.group?.visibility || '--'" severity="secondary" />
            </dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.kind") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">
              <Tag class="justify-self-end" :value="state.group?.kind || '--'" severity="contrast" />
            </dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.currentRole") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">
              <Tag class="justify-self-end" :value="state.group?.current_role || '--'" :severity="state.roleSeverity(state.group?.current_role)" />
            </dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.membersTitle") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">{{ state.members.length }}</dd>
          </div>
          <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-app-border/40 pb-2 last:border-b-0 last:pb-0">
            <dt class="text-xs font-medium uppercase tracking-[0.12em] text-app-text-dim">{{ $t("groups.childrenTitle") }}</dt>
            <dd class="min-w-0 text-right text-sm font-semibold text-app-text">{{ state.childGroups.length }}</dd>
          </div>
        </dl>
      </section>
    </aside>
  </div>
</template>
