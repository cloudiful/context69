<script setup lang="ts">
import Button from "primevue/button";
import Tag from "primevue/tag";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <div class="grid gap-3 xl:max-w-[28rem]">
    <section class="grid gap-3 rounded-[1rem] border border-app-border/65 bg-app-surface-muted/18 p-4">
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
      </dl>
    </section>

    <section class="grid gap-3 rounded-[1rem] border border-app-border/65 bg-app-surface-muted/18 p-4">
      <div class="flex flex-wrap items-center gap-2">
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
