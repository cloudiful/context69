<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import type { GroupMemberResponse, MemberSortBy } from "../../services/api";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";
import AppServerList from "../../components/AppServerList.vue";

const state = useGroupWorkspaceContext();
const { t } = useI18n();
const columns = computed<TableColumn<GroupMemberResponse>[]>(() => [
  { accessorKey: "login_name", header: t("adminUsers.loginName"), enableSorting: true },
  { accessorKey: "display_name", header: t("adminUsers.displayName"), enableSorting: true },
  { accessorKey: "role", header: t("members.role"), enableSorting: true },
  ...(state.canManageGroup ? [{ id: "actions", header: "" }] : []),
]);

const sorting = ref<{ id: string; desc: boolean }[]>([]);

watch(sorting, (value) => {
  const next = value[0];
  if (!next) {
    state.clearMembersSort();
    return;
  }
  if (!["login_name", "display_name", "role"].includes(next.id)) return;
  state.changeMembersSort(next.id as MemberSortBy, next.desc ? "desc" : "asc");
});

watch(() => state.membersSort, (membersSort) => {
  const current = sorting.value[0];
  if (!membersSort) {
    if (current) sorting.value = [];
    return;
  }
  if (current?.id !== membersSort.field || current?.desc !== (membersSort.direction === "desc")) {
    sorting.value = [{ id: membersSort.field, desc: membersSort.direction === "desc" }];
  }
});
</script>

<template>
  <AppServerList
    :pagination="state.membersPage.pagination"
    @update:page="state.changeMembersPage"
    @update:page-size="state.changePageSize"
  >
    <template #toolbar>
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="flex flex-wrap items-center gap-2">
          <p class="text-base font-semibold text-color">{{ $t("groups.membersTitle") }}</p>
          <UInput v-model="state.membersSearch" class="w-56" icon="i-lucide-search" :placeholder="$t('members.searchUserPlaceholder')" />
        </div>
        <UButton v-if="state.canManageGroup" size="sm" @click="state.openCreateMemberDialog">
          {{ $t("members.add") }}
        </UButton>
      </div>
    </template>

    <UTable
      v-model:sorting="sorting"
      class="min-w-0 max-w-full"
      :data="state.members"
      :columns="columns"
      :sorting-options="{ manualSorting: true }"
    >
      <template #role-cell="{ row }"><UBadge :label="row.original.role" :color="state.roleSeverity(row.original.role)" variant="subtle" /></template>
      <template #actions-cell="{ row }">
          <div class="flex gap-2">
            <UButton color="neutral" variant="outline" size="sm" @click="state.openEditMemberDialog(row.original)">
              {{ $t("common.edit") }}
            </UButton>
            <UButton color="error" variant="outline" size="sm" @click="state.confirmRemoveMember(row.original)">
              {{ $t("common.delete") }}
            </UButton>
          </div>
      </template>
    </UTable>
  </AppServerList>
</template>
