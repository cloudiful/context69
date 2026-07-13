<script setup lang="ts">
import { computed } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import type { GroupMemberResponse } from "../../services/api";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
const { t } = useI18n();
const columns = computed<TableColumn<GroupMemberResponse>[]>(() => [
  { accessorKey: "login_name", header: t("adminUsers.loginName") },
  { accessorKey: "display_name", header: t("adminUsers.displayName") },
  { accessorKey: "role", header: t("members.role") },
  ...(state.canManageGroup ? [{ id: "actions", header: "" }] : []),
]);
</script>

<template>
  <section class="grid gap-3">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <p class="text-base font-semibold text-color">{{ $t("groups.membersTitle") }}</p>
      <UButton v-if="state.canManageGroup" size="sm" @click="state.openCreateMemberDialog">
        {{ $t("members.add") }}
      </UButton>
    </div>

    <UTable
      class="min-w-0 max-w-full"
      :data="state.members"
      :columns="columns"
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
  </section>
</template>
