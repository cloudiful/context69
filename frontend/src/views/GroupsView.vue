<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import AppServerList from "../components/AppServerList.vue";
import EntityDialog from "../components/EntityDialog.vue";
import { apiClient, type GroupKind, type GroupResponse, type GroupSortBy, type Visibility } from "../services/api";
import { useErrorToast } from "../composables/use-error-toast";
import { useServerPagination } from "../composables/use-server-pagination";
import { formatTimestamp } from "../utils/format";

const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();
const query = ref("");
const visibilityFilter = ref<Visibility | null>(null);
const kindFilter = ref<GroupKind | null>(null);
const createDialogVisible = ref(false);
const createBusy = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | undefined;

const pageState = useServerPagination<GroupResponse>((request, options) => apiClient.listGroups({
  page: request.page,
  page_size: request.page_size,
  query: query.value.trim() || undefined,
  sort_by: request.sort?.field as GroupSortBy | undefined,
  sort_direction: request.sort?.direction as "asc" | "desc" | undefined,
  visibility: visibilityFilter.value ?? undefined,
  kind: kindFilter.value ?? undefined,
}, options));
const error = pageState.error;
const groups = pageState.items;
const loading = pageState.loading;
const pagination = pageState.pagination;

async function loadGroups() {
  await pageState.load();
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

const visibilityOptions = computed(() => [
  { label: t("groups.allVisibilities"), value: null },
  { label: t("groups.visibilityOptions.private"), value: "private" },
  { label: t("groups.visibilityOptions.public"), value: "public" },
]);

const kindOptions = computed(() => [
  { label: t("groups.allKinds"), value: null },
  { label: t("groups.kindOptions.shared"), value: "shared" },
  { label: t("groups.kindOptions.personal"), value: "personal" },
]);

const columns = computed<TableColumn<GroupResponse>[]>(() => [
  { accessorKey: "group_key", header: t("groups.groupKey"), enableSorting: true },
  { accessorKey: "name", header: t("groups.groupName"), enableSorting: true },
  { accessorKey: "visibility", header: t("groups.visibility") },
  { accessorKey: "kind", header: t("groups.kind") },
  { accessorKey: "current_role", header: t("groups.currentRole") },
  { accessorKey: "created_at", header: t("groups.createdAt"), enableSorting: true },
]);

const sorting = ref<{ id: string; desc: boolean }[]>([]);

watch(sorting, (value) => {
  const next = value[0];
  if (!next) {
    pageState.clearSort();
    return;
  }
  if (!["group_key", "name", "created_at"].includes(next.id)) return;
  pageState.changeSort(next.id, next.desc ? "desc" : "asc");
});

onMounted(() => {
  void loadGroups();
});

watch(error, (cause) => {
  if (cause) showErrorToast(cause, t("groups.loadFailed"));
});

watch(query, () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    pageState.page.value = 1;
    void pageState.load(1);
  }, 250);
});

watch([visibilityFilter, kindFilter], () => {
  pageState.page.value = 1;
  void pageState.load(1);
});

onBeforeUnmount(() => clearTimeout(searchTimer));
</script>

<template>
  <AppServerList
    :loading="loading"
    :pagination="pagination"
    @retry="loadGroups"
    @update:page="pageState.changePage"
    @update:page-size="pageState.changePageSize"
  >
    <template #toolbar>
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex flex-wrap items-center gap-2">
          <UInput v-model="query" class="w-64 max-w-full" icon="i-lucide-search" :placeholder="t('groups.groupName')" />
          <USelect :model-value="visibilityFilter" :items="visibilityOptions" value-key="value" class="w-40" :aria-label="t('groups.visibility')" @update:model-value="visibilityFilter = $event as Visibility | null" />
          <USelect :model-value="kindFilter" :items="kindOptions" value-key="value" class="w-40" :aria-label="t('groups.kind')" @update:model-value="kindFilter = $event as GroupKind | null" />
        </div>
        <UButton @click="createDialogVisible = true">
          {{ t("groups.create") }}
        </UButton>
      </div>
    </template>

    <UTable
      v-model:sorting="sorting"
      class="min-w-0 max-w-full"
      :data="groups"
      :columns="columns"
      :loading="loading"
      :sorting-options="{ manualSorting: true }"
      @select="handleGroupSelect"
    >
      <template #empty>
        {{ t("groups.emptyMessage") }}
      </template>

      <template #visibility-cell="{ row }"><UBadge :label="row.original.visibility" color="neutral" variant="subtle" /></template>
      <template #kind-cell="{ row }"><UBadge :label="row.original.kind" color="neutral" variant="subtle" /></template>
      <template #current_role-cell="{ row }"><UBadge :label="row.original.current_role || '--'" :color="roleSeverity(row.original.current_role)" variant="subtle" /></template>
      <template #created_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.created_at) }}</span></template>
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
  </AppServerList>
</template>
