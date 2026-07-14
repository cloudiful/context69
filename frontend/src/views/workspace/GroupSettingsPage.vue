<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";
import type { Visibility } from "../../services/api";
import MetadataIndexesSection from "../../components/MetadataIndexesSection.vue";
import GroupTranslationSettingsSection from "../../components/GroupTranslationSettingsSection.vue";

const state = useGroupWorkspaceContext();
const { t } = useI18n();
const name = ref("");
const visibility = ref<Visibility>("private");
const visibilityOptions = computed(() => [
  { label: t("groups.visibilityOptions.private"), value: "private" as const },
  { label: t("groups.visibilityOptions.public"), value: "public" as const },
]);
const hasChanges = computed(() => name.value.trim() !== (state.group?.name ?? "") || visibility.value !== state.group?.visibility);

watch(
  () => state.group,
  (group) => {
    name.value = group?.name ?? "";
    visibility.value = group?.visibility ?? "private";
  },
  { immediate: true },
);

function save() {
  if (!name.value.trim() || !hasChanges.value) return;
  void state.saveGroup({ name: name.value.trim(), visibility: visibility.value });
}
</script>

<template>
  <div class="grid gap-3 xl:max-w-[28rem]">
    <UCard>
      <form class="grid gap-3" @submit.prevent="save">
        <dl class="grid gap-2">
        <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-surface pb-2 last:border-b-0 last:pb-0">
          <dt class="text-xs font-medium uppercase tracking-[0.12em] text-muted-color">{{ $t("groups.groupKey") }}</dt>
          <dd class="min-w-0 text-right text-sm font-semibold text-color">{{ state.group?.group_key || state.groupKey }}</dd>
        </div>
        <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-surface pb-2 last:border-b-0 last:pb-0">
          <dt class="text-xs font-medium uppercase tracking-[0.12em] text-muted-color">{{ $t("groups.groupPath") }}</dt>
          <dd class="min-w-0 text-right text-sm font-semibold text-color">{{ state.group?.group_path || state.groupPath }}</dd>
        </div>
        <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-surface pb-2 last:border-b-0 last:pb-0">
          <dt class="text-xs font-medium uppercase tracking-[0.12em] text-muted-color">{{ $t("groups.groupName") }}</dt>
          <dd class="min-w-0">
            <UInput
              v-if="state.canManageGroup"
              v-model="name"
              class="w-48 text-right"
              :aria-label="$t('groups.groupName')"
            />
            <span v-else class="block text-right text-sm font-semibold text-color">{{ state.group?.name || "--" }}</span>
          </dd>
        </div>
        <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-surface pb-2 last:border-b-0 last:pb-0">
          <dt class="text-xs font-medium uppercase tracking-[0.12em] text-muted-color">{{ $t("groups.visibility") }}</dt>
          <dd class="min-w-0">
            <USelect
              v-if="state.canManageGroup"
              v-model="visibility"
              class="w-32"
              :items="visibilityOptions"
              label-key="label"
              value-key="value"
              :aria-label="$t('groups.visibility')"
            />
            <UBadge v-else class="justify-self-end" :label="state.group?.visibility || '--'" color="neutral" />
          </dd>
        </div>
        <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-surface pb-2 last:border-b-0 last:pb-0">
          <dt class="text-xs font-medium uppercase tracking-[0.12em] text-muted-color">{{ $t("groups.kind") }}</dt>
          <dd class="min-w-0 text-right text-sm font-semibold text-color">
            <UBadge class="justify-self-end" :label="state.group?.kind || '--'" color="neutral" />
          </dd>
        </div>
        </dl>

        <div v-if="state.canManageGroup" class="flex justify-end">
          <UButton type="submit" :disabled="state.groupDialogBusy || !name.trim() || !hasChanges">
            {{ $t("common.save") }}
          </UButton>
        </div>
      </form>
    </UCard>

    <MetadataIndexesSection :group-path="state.groupPath" :can-manage="state.canManageGroup" />
    <GroupTranslationSettingsSection :group-path="state.groupPath" :can-manage="state.canManageGroup" />

    <UCard>
      <div class="flex flex-wrap items-center gap-2">
        <UButton v-if="state.canManageGroup" color="neutral" variant="outline" @click="state.openMoveCurrentGroupDialog">
          {{ $t("common.move") }}
        </UButton>
        <UButton v-if="state.canOwnGroup" color="error" @click="state.confirmDeleteGroup">
          {{ $t("common.delete") }}
        </UButton>
      </div>
    </UCard>
  </div>
</template>
