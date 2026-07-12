<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import { useConfirm } from "primevue/useconfirm";
import { useToast } from "primevue/usetoast";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import SourceEditorForm from "../components/SourceEditorForm.vue";
import SourceTable from "../components/SourceTable.vue";
import {
  apiClient,
  type SourceConfigInput,
  type SourceConnectionResponse,
  type SourceStatus,
} from "../services/api";
import { toolSecondaryButtonClass } from "../ui/button-classes";
import { useErrorToast } from "../composables/use-error-toast";

const { t } = useI18n();
const confirm = useConfirm();
const toast = useToast();
const showErrorToast = useErrorToast();

const sources = ref<SourceStatus[]>([]);
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
    sources.value = await apiClient.listSources();
    if (sources.value.length === 0) {
      editorOpen.value = true;
    }
  } catch (error) {
    showErrorToast(error, t("sources.pageLoadFailed"));
  } finally {
    loading.value = false;
  }
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
      severity: "success",
      summary: editingSource.value ? t("sources.form.save") : t("sources.form.create"),
      life: 2500,
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
      severity: "success",
      summary: t("sources.sync"),
      detail: sourceKey,
      life: 2500,
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
  editorOpen.value = sources.value.length === 0;
  editorRevision.value += 1;
}

function deleteSource(sourceKey: string) {
  confirm.require({
    header: t("common.delete"),
    message: t("sources.deleteConfirm"),
    icon: "pi pi-exclamation-triangle",
    rejectProps: {
      label: t("common.cancel"),
      severity: "secondary",
      outlined: true,
    },
    acceptProps: {
      label: t("common.delete"),
      severity: "danger",
    },
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
      severity: "success",
      summary: t("common.delete"),
      detail: sourceKey,
      life: 2500,
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
            :syncing-map="syncingMap"
            :deleting-map="deletingMap"
            @create="startCreate"
            @delete="deleteSource"
            @edit="startEdit"
            @select="startEdit"
            @refresh="loadSources"
            @sync="syncSource"
          />
        </section>

        <AppStateMessage v-if="sources.length === 0" :title="t('sources.emptyTitle')">
          {{ t("sources.emptyMessage") }}
        </AppStateMessage>
      </div>

      <section v-else class="min-h-[calc(100vh-8.25rem)] max-md:min-h-0 xl:min-h-[calc(100vh-8.5rem)]">
        <div class="grid h-full gap-4 rounded-[0.8rem] border border-(--p-content-border-color)/80 bg-(--p-content-hover-background)/36 p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]">
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div>
              <p class="text-base font-semibold text-(--p-text-color)">
                {{ editingSource ? t("sources.editSource") : t("sources.newSource") }}
              </p>
              <p class="text-sm text-(--p-text-muted-color)">
                {{ t("sources.editorDescription") }}
              </p>
            </div>
            <Button
              v-if="sources.length > 0"
              :class="[toolSecondaryButtonClass, 'min-w-24']"
              type="button"
              outlined
              size="small"
              severity="secondary"
              @click="resetEditor"
            >
              {{ t("common.close") }}
            </Button>
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
      </section>
    </AsyncStateBlock>
  </div>
</template>
