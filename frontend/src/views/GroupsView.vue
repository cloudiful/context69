<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import EntityDialog from "../components/EntityDialog.vue";
import { apiClient, type GroupResponse, type Visibility } from "../services/api";
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
  void router.push({ name: "group-overview", params: { groupPath: group.group_path ?? group.group_key } });
}

function handleGroupSelect(_event: Event, row: { original: GroupResponse }) {
  openGroup(row.original);
}

function roleSeverity(role?: string | null) {
  if (role === "owner") return "success";
  if (role === "maintainer") return "info";
  return "neutral";
}

const columns = computed<TableColumn<GroupResponse>[]>(() => [
  { accessorKey: "group_key", header: t("groups.groupKey") },
  { accessorKey: "name", header: t("groups.groupName") },
  { accessorKey: "visibility", header: t("groups.visibility") },
  { accessorKey: "kind", header: t("groups.kind") },
  { accessorKey: "current_role", header: t("groups.currentRole") },
]);

onMounted(() => {
  void loadGroups();
});
</script>

<template>
  <div class="grid min-w-0 gap-2">
    <div class="flex justify-end">
      <UButton @click="createDialogVisible = true">
        {{ t("groups.create") }}
      </UButton>
    </div>

    <UTable
      class="min-w-0 max-w-full"
      :data="groups"
      :columns="columns"
      :loading="loading"
      @select="handleGroupSelect"
    >
      <template #empty>
        {{ t("groups.emptyMessage") }}
      </template>

      <template #visibility-cell="{ row }"><UBadge :label="row.original.visibility" color="neutral" variant="subtle" /></template>
      <template #kind-cell="{ row }"><UBadge :label="row.original.kind" color="neutral" variant="subtle" /></template>
      <template #current_role-cell="{ row }"><UBadge :label="row.original.current_role || '--'" :color="roleSeverity(row.original.current_role)" variant="subtle" /></template>
    </UTable>

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
