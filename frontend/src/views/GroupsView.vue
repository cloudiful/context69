<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "primevue/tag";

import EntityDialog from "../components/EntityDialog.vue";
import { apiClient, type GroupResponse, type Visibility } from "../services/api";
import { toolPrimaryButtonClass } from "../ui/button-classes";
import { useErrorToast } from "../composables/use-error-toast";

const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();
const groups = ref<GroupResponse[]>([]);
const loading = ref(false);
const createDialogVisible = ref(false);
const createBusy = ref(false);

async function loadGroups() {
  loading.value = true;
  try {
    const nextGroups = await apiClient.listGroups();
    groups.value = nextGroups.filter((group) => !group.parent_group_path);
  } catch (error) {
    showErrorToast(error, t("groups.loadFailed"));
  } finally {
    loading.value = false;
  }
}

async function createGroup(payload: { key?: string; name: string; visibility: Visibility }) {
  createBusy.value = true;
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
    showErrorToast(error, t("groups.createFailed"));
  } finally {
    createBusy.value = false;
  }
}

function openGroup(group: GroupResponse) {
  void router.push({ name: "group-detail", params: { groupPath: group.group_path ?? group.group_key } });
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
  <div class="grid gap-2">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <h1 class="section-title">{{ t("groups.title") }}</h1>
      <Button :class="toolPrimaryButtonClass" @click="createDialogVisible = true">
        {{ t("groups.create") }}
      </Button>
    </div>

    <DataTable
      class="app-data-table"
      :value="groups"
      :loading="loading"
      data-key="group_id"
      removable-sort
      row-hover
      resizable-columns
      column-resize-mode="expand"
      scrollable
      state-storage="local"
      state-key="context69:table:groups:v2"
      table-style="min-width: 100%"
      @row-click="openGroup($event.data)"
    >
      <template #empty>
        {{ t("groups.emptyMessage") }}
      </template>

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

    <EntityDialog
      v-model:visible="createDialogVisible"
      :busy="createBusy"
      :show-key="true"
      :title="t('groups.create')"
      :entity-key-label="t('groups.groupKey')"
      :entity-name-label="t('groups.groupName')"
      :submit-label="t('groups.create')"
      @submit="createGroup"
    />
  </div>
</template>
