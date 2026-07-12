<script setup lang="ts">
import { computed, ref } from "vue";
import Button from "primevue/button";
import Checkbox from "primevue/checkbox";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Select from "primevue/select";
import { useI18n } from "vue-i18n";
import AppTag from "./AppTag.vue";
import { apiClient, type CreateMetadataIndexRequest, type MetadataIndexResponse } from "../services/api";

const props = defineProps<{ groupPath: string; canManage: boolean }>();
const { t } = useI18n();
const sourceKey = ref("");
const rows = ref<MetadataIndexResponse[]>([]);
const loading = ref(false);
const dialogVisible = ref(false);
const path = ref("");
const dataType = ref<CreateMetadataIndexRequest["data_type"]>("keyword");
const valueKind = ref<CreateMetadataIndexRequest["value_kind"]>("scalar");
const sortable = ref(false);
const typeOptions = ["keyword", "integer", "float", "boolean", "datetime"];
const kindOptions = ["scalar", "array"];
const canSubmit = computed(() => sourceKey.value.trim() && path.value.trim() && !(valueKind.value === "array" && sortable.value));

async function load() {
  if (!sourceKey.value.trim()) return;
  loading.value = true;
  try { rows.value = await apiClient.listMetadataIndexes(props.groupPath, sourceKey.value.trim()); }
  finally { loading.value = false; }
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
  <section class="grid gap-3 border-t border-app-border/60 pt-4">
    <div class="flex flex-wrap items-end gap-2">
      <label class="grid min-w-56 flex-1 gap-1 text-xs font-medium text-app-text-dim">
        {{ t("metadataIndexes.sourceKey") }}
        <InputText v-model="sourceKey" @keyup.enter="load" />
      </label>
      <Button severity="secondary" variant="outlined" :disabled="loading || !sourceKey.trim()" @click="load">{{ t("common.refresh") }}</Button>
      <Button v-if="canManage" :disabled="!sourceKey.trim()" @click="dialogVisible = true">{{ t("metadataIndexes.add") }}</Button>
    </div>
    <DataTable :value="rows" :loading="loading" size="small" data-key="index_id" :empty-message="t('metadataIndexes.empty')">
      <Column field="path" :header="t('metadataIndexes.path')" />
      <Column field="data_type" :header="t('metadataIndexes.type')" />
      <Column field="value_kind" :header="t('metadataIndexes.kind')" />
      <Column :header="t('metadataIndexes.status')">
        <template #body="{ data }"><AppTag :value="data.status" :severity="data.status === 'ready' ? 'success' : data.status === 'failed' ? 'danger' : 'secondary'" /></template>
      </Column>
      <Column v-if="canManage" :header="t('common.actions')">
        <template #body="{ data }">
          <div class="flex gap-1">
            <Button v-if="data.status === 'failed'" size="small" severity="secondary" variant="outlined" @click="retry(data)">{{ t("common.retry") }}</Button>
            <Button size="small" severity="danger" variant="text" @click="remove(data)">{{ t("common.delete") }}</Button>
          </div>
        </template>
      </Column>
    </DataTable>
    <Dialog v-model:visible="dialogVisible" modal :header="t('metadataIndexes.add')" class="w-[28rem] max-w-[96vw]">
      <form class="grid gap-3" @submit.prevent="create">
        <label class="grid gap-1 text-sm">{{ t("metadataIndexes.path") }}<InputText v-model="path" autofocus /></label>
        <label class="grid gap-1 text-sm">{{ t("metadataIndexes.type") }}<Select v-model="dataType" :options="typeOptions" /></label>
        <label class="grid gap-1 text-sm">{{ t("metadataIndexes.kind") }}<Select v-model="valueKind" :options="kindOptions" /></label>
        <label class="flex items-center gap-2 text-sm"><Checkbox v-model="sortable" binary :disabled="valueKind === 'array'" />{{ t("metadataIndexes.sortable") }}</label>
        <div class="flex justify-end gap-2"><Button severity="secondary" variant="outlined" @click="dialogVisible = false">{{ t("common.cancel") }}</Button><Button type="submit" :disabled="!canSubmit">{{ t("common.create") }}</Button></div>
      </form>
    </Dialog>
  </section>
</template>
