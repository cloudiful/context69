<script setup lang="ts">
import { ref } from "vue";
import ProjectFilesPanel from "../../components/ProjectFilesPanel.vue";
import Button from "primevue/button";
import Tag from "primevue/tag";

import AppMdiIcon from "../../components/AppMdiIcon.vue";
import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";
import { controlButtonClass } from "../../ui/button-classes";

const state = useGroupWorkspaceContext();
const overviewCollapsed = ref(false);
const mdiChevronLeft = "M15.41,16.59L10.83,12L15.41,7.41L14,6L8,12L14,18L15.41,16.59Z";
const mdiChevronRight = "M8.59,16.59L13.17,12L8.59,7.41L10,6L16,12L10,18L8.59,16.59Z";
</script>

<template>
  <div
    class="relative grid h-full min-h-0 gap-4"
    :class="overviewCollapsed ? 'lg:grid-cols-1' : 'lg:grid-cols-[minmax(0,1.7fr)_16rem]'"
  >
    <section class="grid h-full min-h-0 min-w-0">
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

    <aside v-if="!overviewCollapsed" class="grid self-start gap-3">
      <section class="grid gap-3 rounded-[1rem] border border-app-border/65 bg-app-surface-muted/18 p-4">
        <div class="flex items-center justify-between gap-2">
          <p class="section-title">{{ $t("groups.tabs.overview") }}</p>
          <Button
            :class="[controlButtonClass, 'h-8 w-8 shrink-0 px-0']"
            type="button"
            :aria-label="$t('sidebar.collapse')"
            :title="$t('sidebar.collapse')"
            :aria-expanded="true"
            @click="overviewCollapsed = true"
          >
            <AppMdiIcon :path="mdiChevronRight" class="h-4 w-4" />
          </Button>
        </div>
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

    <Button
      v-else
      :class="[controlButtonClass, 'absolute right-0 top-0 z-10 h-8 w-8 px-0']"
      type="button"
      :aria-label="$t('sidebar.expand')"
      :title="$t('sidebar.expand')"
      :aria-expanded="false"
      @click="overviewCollapsed = false"
    >
      <AppMdiIcon :path="mdiChevronLeft" class="h-4 w-4" />
    </Button>
  </div>
</template>
