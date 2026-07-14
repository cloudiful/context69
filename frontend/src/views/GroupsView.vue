<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
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
const page = ref(1);
const pageSize = ref(50);
const total = ref(0);
const query = ref("");
const createDialogVisible = ref(false);
const createBusy = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | undefined;

async function loadGroups() {
  loading.value = true;
  try {
    const response = await apiClient.listGroups({
      page: page.value,
      page_size: pageSize.value,
      query: query.value.trim() || undefined,
    });
    groups.value = response.items;
    total.value = response.total;
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

watch(query, () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    page.value = 1;
    void loadGroups();
  }, 250);
});

onBeforeUnmount(() => clearTimeout(searchTimer));
</script>

<template>
  <div class="grid min-w-0 gap-2">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <UInput v-model="query" class="w-64 max-w-full" icon="i-lucide-search" :placeholder="t('groups.groupName')" />
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

    <UPagination
      v-if="total > pageSize"
      v-model:page="page"
      :items-per-page="pageSize"
      :total="total"
      class="justify-end"
      @update:page="loadGroups"
    />

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
