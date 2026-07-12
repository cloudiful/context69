<script setup lang="ts">
import { computed, ref, watch } from "vue";
import Button from "primevue/button";
import InputText from "primevue/inputtext";
import Select from "primevue/select";
import Tag from "../../components/AppTag.vue";
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
    <section class="grid gap-3 rounded-[1rem] border border-surface bg-emphasis p-4">
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
            <InputText
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
            <Select
              v-if="state.canManageGroup"
              v-model="visibility"
              class="w-32"
              :options="visibilityOptions"
              option-label="label"
              option-value="value"
              :aria-label="$t('groups.visibility')"
            />
            <Tag v-else class="justify-self-end" :value="state.group?.visibility || '--'" severity="secondary" />
          </dd>
        </div>
        <div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b border-surface pb-2 last:border-b-0 last:pb-0">
          <dt class="text-xs font-medium uppercase tracking-[0.12em] text-muted-color">{{ $t("groups.kind") }}</dt>
          <dd class="min-w-0 text-right text-sm font-semibold text-color">
            <Tag class="justify-self-end" :value="state.group?.kind || '--'" severity="contrast" />
          </dd>
        </div>
        </dl>

        <div v-if="state.canManageGroup" class="flex justify-end">
          <Button type="submit" :disabled="state.groupDialogBusy || !name.trim() || !hasChanges">
            {{ $t("common.save") }}
          </Button>
        </div>
      </form>
    </section>

    <MetadataIndexesSection :group-path="state.groupPath" :can-manage="state.canManageGroup" />
    <GroupTranslationSettingsSection :group-path="state.groupPath" :can-manage="state.canManageGroup" />

    <section class="grid gap-3 rounded-[1rem] border border-surface bg-emphasis p-4">
      <div class="flex flex-wrap items-center gap-2">
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
