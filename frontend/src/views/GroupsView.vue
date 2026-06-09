<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Message from "primevue/message";
import Tag from "primevue/tag";

import AppPanel from "../components/AppPanel.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import EntityDialog from "../components/EntityDialog.vue";
import { apiClient, type GroupResponse } from "../services/api";

const router = useRouter();
const { t } = useI18n();
const groups = ref<GroupResponse[]>([]);
const loading = ref(false);
const errorMessage = ref("");
const dialogError = ref("");
const createDialogVisible = ref(false);
const createBusy = ref(false);

async function loadGroups() {
  loading.value = true;
  try {
    errorMessage.value = "";
    groups.value = await apiClient.listGroups();
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : t("groups.loadFailed");
  } finally {
    loading.value = false;
  }
}

async function createGroup(payload: { key?: string; name: string; visibility: "private" | "public" }) {
  createBusy.value = true;
  dialogError.value = "";
  try {
    await apiClient.createGroup({
      group_key: payload.key?.trim() ?? "",
      name: payload.name.trim(),
      visibility: payload.visibility,
      kind: "shared",
    });
    createDialogVisible.value = false;
    await loadGroups();
  } catch (error) {
    dialogError.value = error instanceof Error ? error.message : t("groups.createFailed");
  } finally {
    createBusy.value = false;
  }
}

function openGroup(group: GroupResponse) {
  void router.push({ name: "group-detail", params: { groupKey: group.group_key } });
}

function roleSeverity(role?: string | null) {
  if (role === "owner") return "success";
  if (role === "maintainer") return "info";
  return "secondary";
}

onMounted(() => {
  void loadGroups();
});
</script>

<template>
  <div class="workspace-page">
    <AppPanel :title="t('groups.title')">
      <template #actions>
        <Button class="tool-action-primary" @click="createDialogVisible = true">
          {{ t("groups.create") }}
        </Button>
      </template>

      <Message v-if="errorMessage" severity="error" :closable="false">{{ errorMessage }}</Message>

      <DataTable
        v-if="groups.length > 0"
        :value="groups"
        data-key="group_id"
        removable-sort
        scrollable
        size="small"
        table-style="min-width: 100%"
        @row-click="openGroup($event.data)"
      >
        <Column field="group_key" :header="t('groups.groupKey')" sortable />
        <Column field="name" :header="t('groups.groupName')" sortable />
        <Column field="visibility" :header="t('groups.visibility')" sortable>
          <template #body="{ data }">
            <Tag :value="data.visibility" severity="secondary" />
          </template>
        </Column>
        <Column field="kind" :header="t('groups.kind')" sortable>
          <template #body="{ data }">
            <Tag :value="data.kind" severity="contrast" />
          </template>
        </Column>
        <Column field="current_role" :header="t('groups.currentRole')" sortable>
          <template #body="{ data }">
            <Tag :value="data.current_role || '--'" :severity="roleSeverity(data.current_role)" />
          </template>
        </Column>
      </DataTable>

      <AppStateMessage v-else-if="!loading" :title="t('groups.emptyTitle')">
        {{ t("groups.emptyMessage") }}
      </AppStateMessage>

      <EntityDialog
        v-model:visible="createDialogVisible"
        :busy="createBusy"
        :error="dialogError"
        :show-key="true"
        :title="t('groups.create')"
        :entity-key-label="t('groups.groupKey')"
        :entity-name-label="t('groups.groupName')"
        :submit-label="t('groups.create')"
        @submit="createGroup"
      />
    </AppPanel>
  </div>
</template>
