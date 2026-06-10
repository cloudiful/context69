<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import Message from "primevue/message";

import AppStateMessage from "./AppStateMessage.vue";
import SourceEditorForm from "./SourceEditorForm.vue";
import SourceTable from "./SourceTable.vue";
import { apiClient, type SourceConfigInput, type SourceConnectionResponse, type SourceStatus } from "../services/api";

const props = defineProps<{
  groupKey: string;
  projectKey: string;
  canManage?: boolean;
}>();

const { t } = useI18n();
const sources = ref<SourceStatus[]>([]);
const connections = ref<SourceConnectionResponse[]>([]);
const loading = ref(false);
const formBusy = ref(false);
const pageError = ref("");
const formError = ref("");
const editingSource = ref<SourceStatus | null>(null);
const editorOpen = ref(false);
const editorRevision = ref(0);
const syncingMap = ref<Record<string, boolean>>({});
const deletingMap = ref<Record<string, boolean>>({});
const errorMap = ref<Record<string, string>>({});

async function loadSources() {
  loading.value = true;
  try {
    pageError.value = "";
    connections.value = await apiClient.listSourceConnections();
    sources.value = await apiClient.listProjectSources(props.groupKey, props.projectKey);
  } catch (error) {
    pageError.value = error instanceof Error ? error.message : t("sources.pageLoadFailed");
  } finally {
    loading.value = false;
  }
}

async function saveSource(payload: SourceConfigInput) {
  formBusy.value = true;
  formError.value = "";
  try {
    if (editingSource.value) {
      await apiClient.updateProjectSource(props.groupKey, props.projectKey, editingSource.value.source_key, payload);
    } else {
      await apiClient.createProjectSource(props.groupKey, props.projectKey, payload);
    }
    await loadSources();
    resetEditor();
  } catch (error) {
    formError.value = error instanceof Error ? error.message : t("sources.updateFailed");
  } finally {
    formBusy.value = false;
  }
}

async function syncSource(sourceKey: string) {
  syncingMap.value = { ...syncingMap.value, [sourceKey]: true };
  try {
    await apiClient.syncProjectSource(props.groupKey, props.projectKey, sourceKey);
    await loadSources();
  } catch (error) {
    errorMap.value = { ...errorMap.value, [sourceKey]: error instanceof Error ? error.message : t("sources.syncFailed") };
  } finally {
    syncingMap.value = { ...syncingMap.value, [sourceKey]: false };
  }
}

async function deleteSource(sourceKey: string) {
  deletingMap.value = { ...deletingMap.value, [sourceKey]: true };
  try {
    await apiClient.deleteProjectSource(props.groupKey, props.projectKey, sourceKey);
    await loadSources();
    if (editingSource.value?.source_key === sourceKey) {
      resetEditor();
    }
  } catch (error) {
    errorMap.value = { ...errorMap.value, [sourceKey]: error instanceof Error ? error.message : t("sources.deleteFailed") };
  } finally {
    deletingMap.value = { ...deletingMap.value, [sourceKey]: false };
  }
}

function startCreate() {
  editingSource.value = null;
  editorOpen.value = true;
  formError.value = "";
  editorRevision.value += 1;
}

function startEdit(source: SourceStatus) {
  editingSource.value = source;
  editorOpen.value = true;
  formError.value = "";
}

function resetEditor() {
  editingSource.value = null;
  editorOpen.value = false;
  formError.value = "";
  editorRevision.value += 1;
}

onMounted(() => {
  void loadSources();
});
</script>

<template>
  <div class="workspace-block">
    <Message v-if="pageError" severity="error" :closable="false">{{ pageError }}</Message>

    <div v-if="!editorOpen">
      <SourceTable
        :can-manage="props.canManage"
        :sources="sources"
        :syncing-map="syncingMap"
        :deleting-map="deletingMap"
        :error-map="errorMap"
        @create="startCreate"
        @delete="deleteSource"
        @edit="startEdit"
        @select="startEdit"
        @refresh="loadSources"
        @sync="syncSource"
      />
      <AppStateMessage v-if="sources.length === 0" :title="t('sources.emptyTitle')">
        {{ t("sources.emptyMessage") }}
      </AppStateMessage>
    </div>

    <div v-else-if="props.canManage" class="app-form-block">
      <div class="workspace-block-header">
        <p class="section-title">{{ editingSource ? t("sources.editSource") : t("sources.newSource") }}</p>
      </div>
      <Message v-if="formError" severity="error" :closable="false">{{ formError }}</Message>
      <SourceEditorForm
        :key="editingSource?.source_key ?? `new-${editorRevision}`"
        :busy="formBusy"
        :connections="connections"
        :source="editingSource"
        @cancel="resetEditor"
        @save="saveSource"
      />
    </div>
  </div>
</template>
