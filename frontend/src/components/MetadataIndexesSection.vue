<script setup lang="ts">
import { computed, ref } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import { apiClient, type CreateMetadataIndexRequest, type MetadataIndexResponse, type Pagination } from "../services/api";
import TablePagination from "./TablePagination.vue";

const props = defineProps<{ groupPath: string; canManage: boolean }>();
const { t } = useI18n();
const sourceKey = ref("");
const rows = ref<MetadataIndexResponse[]>([]);
const page = ref(1);
const pageSize = ref(50);
const pagination = ref<Pagination>({ page: 1, page_size: 50, total: 0, total_pages: 0 });
const loading = ref(false);
const dialogVisible = ref(false);
const path = ref("");
const dataType = ref<CreateMetadataIndexRequest["data_type"]>("keyword");
const valueKind = ref<CreateMetadataIndexRequest["value_kind"]>("scalar");
const sortable = ref(false);
const typeOptions = ["keyword", "integer", "float", "boolean", "datetime"];
const kindOptions = ["scalar", "array"];
const canSubmit = computed(() => sourceKey.value.trim() && path.value.trim() && !(valueKind.value === "array" && sortable.value));
const columns = computed<TableColumn<MetadataIndexResponse>[]>(() => [
  { accessorKey: "path", header: t("metadataIndexes.path") },
  { accessorKey: "data_type", header: t("metadataIndexes.type") },
  { accessorKey: "value_kind", header: t("metadataIndexes.kind") },
  { id: "status", header: t("metadataIndexes.status") },
  ...(props.canManage ? [{ id: "actions", header: t("common.actions") }] : []),
]);

async function load() {
  if (!sourceKey.value.trim()) return;
  loading.value = true;
  try {
    const response = await apiClient.listMetadataIndexes(props.groupPath, sourceKey.value.trim(), {
      page: page.value,
      pageSize: pageSize.value,
    });
    rows.value = response.items;
    pagination.value = response.pagination;
  }
  finally { loading.value = false; }
}

function changePage(value: number) {
  page.value = value;
  void load();
}

function changePageSize(value: number) {
  if (pageSize.value === value) return;
  pageSize.value = value;
  page.value = 1;
  void load();
}

async function create() {
  if (!canSubmit.value) return;
  await apiClient.createMetadataIndex(props.groupPath, sourceKey.value.trim(), {
    path: path.value.trim(), data_type: dataType.value, value_kind: valueKind.value, sortable: sortable.value,
  });
  dialogVisible.value = false;
  path.value = "";
  await load();
}

async function retry(row: MetadataIndexResponse) { await apiClient.retryMetadataIndex(props.groupPath, row.index_id); await load(); }
async function remove(row: MetadataIndexResponse) { await apiClient.deleteMetadataIndex(props.groupPath, row.index_id); await load(); }
</script>

<template>
  <section class="grid gap-3 border-t border-surface pt-4">
    <div class="flex flex-wrap items-end gap-2">
      <label class="grid min-w-56 flex-1 gap-1 text-xs font-medium text-muted-color">
        {{ t("metadataIndexes.sourceKey") }}
        <UInput v-model="sourceKey" @keyup.enter="load" />
      </label>
      <UButton color="neutral" variant="outline" :disabled="loading || !sourceKey.trim()" @click="load">{{ t("common.refresh") }}</UButton>
      <UButton v-if="canManage" :disabled="!sourceKey.trim()" @click="dialogVisible = true">{{ t("metadataIndexes.add") }}</UButton>
    </div>
    <UTable class="min-w-0 max-w-full" :data="rows" :columns="columns" :loading="loading" :empty="t('metadataIndexes.empty')">
      <template #status-cell="{ row }"><UBadge :label="row.original.status" :color="row.original.status === 'ready' ? 'success' : row.original.status === 'failed' ? 'error' : 'neutral'" variant="subtle" /></template>
      <template #actions-cell="{ row }">
          <div class="flex gap-1">
            <UButton v-if="row.original.status === 'failed'" size="sm" color="neutral" variant="outline" @click="retry(row.original)">{{ t("common.retry") }}</UButton>
            <UButton size="sm" color="error" variant="ghost" @click="remove(row.original)">{{ t("common.delete") }}</UButton>
          </div>
      </template>
    </UTable>
    <TablePagination
      :pagination="pagination"
      @update:page="changePage"
      @update:page-size="changePageSize"
    />
    <UModal v-model:open="dialogVisible"  :title="t('metadataIndexes.add')" class="w-[28rem] max-w-[96vw]">
    <template #body>
<form class="grid gap-3" @submit.prevent="create">
        <label class="grid gap-1 text-sm">{{ t("metadataIndexes.path") }}<UInput v-model="path" autofocus /></label>
        <label class="grid gap-1 text-sm">{{ t("metadataIndexes.type") }}<USelect v-model="dataType" :items="typeOptions" /></label>
        <label class="grid gap-1 text-sm">{{ t("metadataIndexes.kind") }}<USelect v-model="valueKind" :items="kindOptions" /></label>
        <label class="flex items-center gap-2 text-sm"><UCheckbox v-model="sortable" binary :disabled="valueKind === 'array'" />{{ t("metadataIndexes.sortable") }}</label>
        <div class="flex justify-end gap-2"><UButton color="neutral" variant="outline" @click="dialogVisible = false">{{ t("common.cancel") }}</UButton><UButton type="submit" :disabled="!canSubmit">{{ t("common.create") }}</UButton></div>
      </form>
    </template>
    </UModal>
  </section>
</template>
