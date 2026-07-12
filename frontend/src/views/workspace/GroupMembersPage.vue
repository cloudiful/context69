<script setup lang="ts">
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "../../components/AppTag.vue";

import { useGroupWorkspaceContext } from "../../composables/group-workspace-context";

const state = useGroupWorkspaceContext();
</script>

<template>
  <section class="grid gap-3">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <p class="text-base font-semibold text-(--p-text-color)">{{ $t("groups.membersTitle") }}</p>
      <Button v-if="state.canManageGroup" :label="$t('members.add')" size="small" @click="state.openCreateMemberDialog" />
    </div>

    <DataTable
      :value="state.members"
      data-key="user_id"
      resizable-columns
      column-resize-mode="expand"
      scrollable
      state-storage="local"
      state-key="context69:table:group-members:v2"
      table-class="min-w-full"
    >
      <Column field="login_name" :header="$t('adminUsers.loginName')" />
      <Column field="display_name" :header="$t('adminUsers.displayName')" />
      <Column field="role" :header="$t('members.role')">
        <template #body="{ data }">
          <Tag :value="data.role" :severity="state.roleSeverity(data.role)" />
        </template>
      </Column>
      <Column v-if="state.canManageGroup" :header="$t('common.edit')">
        <template #body="{ data }">
          <div class="flex gap-2">
            <Button severity="secondary" variant="outlined" size="small" @click="state.openEditMemberDialog(data)">
              {{ $t("common.edit") }}
            </Button>
            <Button severity="danger" variant="outlined" size="small" @click="state.confirmRemoveMember(data)">
              {{ $t("common.delete") }}
            </Button>
          </div>
        </template>
      </Column>
    </DataTable>
  </section>
</template>
