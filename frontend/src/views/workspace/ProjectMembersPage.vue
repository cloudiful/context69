<script setup lang="ts">
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Message from "primevue/message";
import Tag from "primevue/tag";

import { useProjectWorkspaceContext } from "../../composables/project-workspace-context";

const state = useProjectWorkspaceContext();
</script>

<template>
  <section class="workspace-block">
    <div class="workspace-block-header">
      <div>
        <p class="section-title">{{ $t("project.membersTitle") }}</p>
      </div>
      <Button
        v-if="state.canManageProject"
        :label="$t('members.add')"
        size="small"
        @click="state.openCreateMemberDialog"
      />
    </div>

    <Message v-if="state.memberError" severity="error" :closable="false">{{ state.memberError }}</Message>

    <DataTable class="app-data-table" :value="state.members" data-key="user_id" scrollable table-style="min-width: 100%">
      <Column field="login_name" :header="$t('adminUsers.loginName')" />
      <Column field="display_name" :header="$t('adminUsers.displayName')" />
      <Column field="role" :header="$t('members.role')">
        <template #body="{ data }">
          <Tag :value="data.role" :severity="state.roleSeverity(data.role)" />
        </template>
      </Column>
      <Column v-if="state.canManageProject" :header="$t('common.edit')">
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
