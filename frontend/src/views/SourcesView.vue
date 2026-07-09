<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Message from "primevue/message";
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

const { t } = useI18n();
const confirm = useConfirm();
const toast = useToast();

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
    await loadConnections();
    sources.value = await apiClient.listSources();
    if (sources.value.length === 0) {
      editorOpen.value = true;
    }
  } catch (error) {
    pageError.value = error instanceof Error ? error.message : t("sources.pageLoadFailed");
  } finally {
    loading.value = false;
  }
}

async function loadConnections() {
  try {
    connections.value = await apiClient.listSourceConnections();
  } catch (error) {
    pageError.value = error instanceof Error ? error.message : t("sources.loadConnectionsFailed");
  }
}

async function saveSource(payload: SourceConfigInput) {
  formBusy.value = true;
  formError.value = "";

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
    formError.value = error instanceof Error
      ? error.message
      : editingSource.value
        ? t("sources.updateFailed")
        : t("sources.createFailed");
  } finally {
    formBusy.value = false;
  }
}

async function syncSource(sourceKey: string) {
  syncingMap.value = {
    ...syncingMap.value,
    [sourceKey]: true,
  };

  errorMap.value = {
    ...errorMap.value,
    [sourceKey]: "",
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
    errorMap.value = {
      ...errorMap.value,
      [sourceKey]: error instanceof Error ? error.message : t("sources.syncFailed"),
    };
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
  editorOpen.value = sources.value.length === 0;
  formError.value = "";
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
  errorMap.value = {
    ...errorMap.value,
    [sourceKey]: "",
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
    errorMap.value = {
      ...errorMap.value,
      [sourceKey]: error instanceof Error ? error.message : t("sources.deleteFailed"),
    };
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
  <div class="sources-page">
    <AsyncStateBlock
      :loading="loading"
      :loading-title="t('sources.pollingTitle')"
      :loading-message="t('sources.pollingMessage')"
      :error="pageError"
    >
      <div v-if="!editorOpen" class="sources-shell">
        <section class="sources-section sources-table-section">
          <SourceTable
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
        </section>

        <AppStateMessage v-if="sources.length === 0" :title="t('sources.emptyTitle')">
          {{ t("sources.emptyMessage") }}
        </AppStateMessage>
      </div>

      <section v-else class="sources-editor-screen">
        <div class="sources-editor-panel app-form-block">
          <div class="sources-editor-actions">
            <div>
              <p class="section-title">
                {{ editingSource ? t("sources.editSource") : t("sources.newSource") }}
              </p>
              <p class="sources-editor-note">
                {{ t("sources.editorDescription") }}
              </p>
            </div>
            <Button
              v-if="sources.length > 0"
              :class="[toolSecondaryButtonClass, 'sources-editor-close']"
              type="button"
              @click="resetEditor"
            >
              {{ t("common.close") }}
            </Button>
          </div>

          <Message v-if="formError" severity="error" :closable="false">
            {{ formError }}
          </Message>

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
