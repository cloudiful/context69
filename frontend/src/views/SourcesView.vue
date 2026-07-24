<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import SourceEditorForm from "../components/SourceEditorForm.vue";
import SourceTable from "../components/SourceTable.vue";
import {
  apiClient,
  type SourceConfigInput,
  type SourceConnectionResponse,
  type Pagination,
  type SourceStatus,
} from "../services/api";
import { useErrorToast } from "../composables/use-error-toast";
import { useAppConfirm } from "../composables/use-app-confirm";

const { t } = useI18n();
const confirm = useAppConfirm();
const toast = useToast();
const showErrorToast = useErrorToast();

const sources = ref<SourceStatus[]>([]);
const sourcePage = ref(1);
const sourcePageSize = ref(50);
const sourceQuery = ref("");
const sourcePagination = ref<Pagination>({ page: 1, page_size: 50, total: 0, total_pages: 0 });
const connections = ref<SourceConnectionResponse[]>([]);
const loading = ref(false);
const formBusy = ref(false);
const editingSource = ref<SourceStatus | null>(null);
const editorOpen = ref(false);
const editorRevision = ref(0);
const syncingMap = ref<Record<string, boolean>>({});
const deletingMap = ref<Record<string, boolean>>({});

async function loadSources() {
  loading.value = true;

  try {
    await loadConnections();
    const response = await apiClient.listSources({
      page: sourcePage.value,
      pageSize: sourcePageSize.value,
      query: sourceQuery.value,
    });
    sources.value = response.items;
    sourcePagination.value = response.pagination;
    if (response.pagination.total === 0) {
      editorOpen.value = true;
    }
  } catch (error) {
    showErrorToast(error, t("sources.pageLoadFailed"));
  } finally {
    loading.value = false;
  }
}

function changeSourcePage(page: number) {
  sourcePage.value = page;
  void loadSources();
}

function changeSourcePageSize(value: number) {
  if (sourcePageSize.value === value) return;
  sourcePageSize.value = value;
  sourcePage.value = 1;
  void loadSources();
}

function updateSourceQuery(value: string) {
  sourceQuery.value = value;
  sourcePage.value = 1;
  void loadSources();
}

async function loadConnections() {
  try {
    connections.value = await apiClient.listSourceConnections();
  } catch (error) {
    showErrorToast(error, t("sources.loadConnectionsFailed"));
  }
}

async function saveSource(payload: SourceConfigInput) {
  formBusy.value = true;

  try {
    if (editingSource.value) {
      await apiClient.updateSource(editingSource.value.source_key, payload);
    } else {
      await apiClient.createSource(payload);
    }

    await loadSources();
    toast.add({
      color: "success",
      title: editingSource.value ? t("sources.form.save") : t("sources.form.create"),
      duration: 2500,
    });
    resetEditor();
  } catch (error) {
    showErrorToast(error, editingSource.value ? t("sources.updateFailed") : t("sources.createFailed"));
  } finally {
    formBusy.value = false;
  }
}

async function syncSource(sourceKey: string) {
  syncingMap.value = {
    ...syncingMap.value,
    [sourceKey]: true,
  };

  try {
    await apiClient.syncSource(sourceKey);
    await loadSources();
    toast.add({
      color: "success",
      title: t("sources.sync"),
      description: sourceKey,
      duration: 2500,
    });
  } catch (error) {
    showErrorToast(error, t("sources.syncFailed"));
  } finally {
    syncingMap.value = {
      ...syncingMap.value,
      [sourceKey]: false,
    };
  }
}

function startCreate() {
  editingSource.value = null;
  editorOpen.value = true;
  editorRevision.value += 1;
}

function startEdit(source: SourceStatus) {
  editingSource.value = source;
  editorOpen.value = true;
}

function resetEditor() {
  editingSource.value = null;
  editorOpen.value = sourcePagination.value.total === 0;
  editorRevision.value += 1;
}

function deleteSource(sourceKey: string) {
  confirm.require({
    header: t("common.delete"),
    message: t("sources.deleteConfirm"),
    rejectLabel: t("common.cancel"),
    acceptLabel: t("common.delete"),
    accept: () => {
      void deleteSourceConfirmed(sourceKey);
    },
  });
}

async function deleteSourceConfirmed(sourceKey: string) {
  deletingMap.value = {
    ...deletingMap.value,
    [sourceKey]: true,
  };

  try {
    await apiClient.deleteSource(sourceKey);
    if (editingSource.value?.source_key === sourceKey) {
      resetEditor();
    }
    await loadSources();
    toast.add({
      color: "success",
      title: t("common.delete"),
      description: sourceKey,
      duration: 2500,
    });
  } catch (error) {
    showErrorToast(error, t("sources.deleteFailed"));
  } finally {
    deletingMap.value = {
      ...deletingMap.value,
      [sourceKey]: false,
    };
  }
}

onMounted(async () => {
  await loadSources();
});
</script>

<template>
  <div class="grid gap-2">
    <AsyncStateBlock
      :loading="loading"
      :loading-title="t('sources.pollingTitle')"
      :loading-message="t('sources.pollingMessage')"
    >
      <div v-if="!editorOpen" class="grid gap-2">
        <section class="grid min-w-0 gap-2">
          <SourceTable
            :sources="sources"
            :pagination="sourcePagination"
            :query="sourceQuery"
            :loading="loading"
            :syncing-map="syncingMap"
            :deleting-map="deletingMap"
            @create="startCreate"
            @delete="deleteSource"
            @edit="startEdit"
            @select="startEdit"
            @refresh="loadSources"
            @page="changeSourcePage"
            @page-size="changeSourcePageSize"
            @update:query="updateSourceQuery"
            @sync="syncSource"
          />
        </section>

        <UAlert
          v-if="sources.length === 0"
          variant="subtle"
          :title="t('sources.emptyTitle')"
          :description="t('sources.emptyMessage')"
        />
      </div>

      <section v-else class="min-h-[calc(100vh-8.25rem)] max-md:min-h-0 xl:min-h-[calc(100vh-8.5rem)]">
        <UCard class="h-full">
          <div class="grid gap-4">
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div>
              <p class="text-base font-semibold text-color">
                {{ editingSource ? t("sources.editSource") : t("sources.newSource") }}
              </p>
              <p class="text-sm text-muted-color">
                {{ t("sources.editorDescription") }}
              </p>
            </div>
            <UButton
              v-if="sources.length > 0"
              class="min-w-24"
              type="button"
              variant="outline"
              size="sm"
              color="neutral"
              @click="resetEditor"
            >
              {{ t("common.close") }}
            </UButton>
          </div>

          <SourceEditorForm
            :key="editingSource?.source_key ?? `new-${editorRevision}`"
            :busy="formBusy"
            :connections="connections"
            :source="editingSource"
            @cancel="resetEditor"
            @save="saveSource"
          />
          </div>
        </UCard>
      </section>
    </AsyncStateBlock>
  </div>
</template>
