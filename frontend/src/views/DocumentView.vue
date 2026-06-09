<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Card from "primevue/card";
import Tag from "primevue/tag";

import AppRecordCard from "../components/AppRecordCard.vue";
import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import AppInfoCard from "../components/AppInfoCard.vue";
import AppPanel from "../components/AppPanel.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import { ApiError, apiClient, type DocumentResponse } from "../services/api";
import { formatDate, formatJson, formatTimestamp } from "../utils/format";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const documentData = ref<DocumentResponse | null>(null);
const loading = ref(false);
const errorMessage = ref("");
const notFound = ref(false);
const expanded = ref(false);

let controller: AbortController | null = null;

const visibleChunks = computed(() => {
  if (!documentData.value) {
    return [];
  }

  return expanded.value ? documentData.value.chunks : documentData.value.chunks.slice(0, 3);
});

const libraryRoute = computed(() => {
  if (!documentData.value?.is_library_file || !documentData.value.library_file_id) {
    return null;
  }

  return {
    name: "library",
    query: {
      file: documentData.value.library_file_id,
    },
  };
});

async function loadDocument() {
  const documentId = Number.parseInt(String(route.params.id), 10);

  if (!Number.isFinite(documentId)) {
    notFound.value = true;
    errorMessage.value = t("document.invalidId");
    return;
  }

  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  notFound.value = false;
  errorMessage.value = "";

  try {
    documentData.value = await apiClient.getDocument(documentId, {
      signal: controller.signal,
    });
  } catch (error) {
    documentData.value = null;
    notFound.value = error instanceof ApiError && error.status === 404;
    errorMessage.value = error instanceof Error ? error.message : t("document.loadFailed");
  } finally {
    loading.value = false;
  }
}

function openSourceTarget() {
  if (libraryRoute.value) {
    void router.push(libraryRoute.value);
    return;
  }

  if (documentData.value?.source_uri) {
    window.open(documentData.value.source_uri, "_blank", "noopener,noreferrer");
  }
}

onMounted(() => {
  void loadDocument();
});

watch(
  () => route.params.id,
  () => {
    expanded.value = false;
    void loadDocument();
  },
);

onBeforeUnmount(() => {
  controller?.abort();
});
</script>

<template>
  <AppPanel :title="t('document.title')">
    <AsyncStateBlock
      :loading="loading"
      loading-test-id="document-loading"
      :loading-title="t('common.loading')"
      :loading-message="t('document.loadingMessage')"
      :error="errorMessage"
      :error-title="notFound ? '404' : t('common.error')"
    >
      <div v-if="documentData" class="document-layout">
        <div class="document-main">
          <AppRecordCard
            :title="documentData.title"
            :subtitle="formatDate(documentData.published_at)"
          >
            <template #tags>
              <div class="flex flex-wrap items-center gap-2">
                <Tag class="tool-chip" :value="documentData.source_key" severity="secondary" />
                <Tag class="tool-chip" :value="documentData.external_id" severity="secondary" />
              </div>
            </template>

            <template #meta>
              <p v-if="documentData.library_path" class="app-record-header-note">
                {{ documentData.library_path }}
                <span v-if="documentData.library_section_label"> · {{ documentData.library_section_label }}</span>
              </p>
            </template>
          </AppRecordCard>

          <div class="document-chunks">
            <Card
              v-for="chunk in visibleChunks"
              :key="chunk.chunk_id"
              class="document-chunk-card"
            >
              <template #content>
                <p class="document-chunk-label">
                  {{ t("document.chunkLabel", { index: chunk.chunk_index }) }}
                </p>
                <pre class="content-pre">{{ chunk.text }}</pre>
              </template>
            </Card>

            <AppStateMessage
              v-if="documentData.chunks.length === 0"
              severity="secondary"
              :title="t('document.noBodyChunksTitle')"
            >
              {{ t("document.noBodyChunksMessage") }}
            </AppStateMessage>
          </div>

          <Button
            v-if="documentData.chunks.length > 3"
            class="tool-action"
            type="button"
            severity="secondary"
            variant="outlined"
            :label="expanded ? t('document.collapse') : t('document.showAll', { count: documentData.chunks.length })"
            @click="expanded = !expanded"
          />
        </div>

        <aside class="document-sidebar">
          <AppInfoCard :label="t('document.published')" :value="formatDate(documentData.published_at)" />
          <AppInfoCard :label="t('document.updated')" :value="formatTimestamp(documentData.updated_at)" />
          <AppInfoCard :label="t('document.sourceLink')">
            <Button
              class="tool-action"
              severity="secondary"
              variant="outlined"
              :label="libraryRoute ? t('document.openLibraryFile') : t('document.openOrigin')"
              @click="openSourceTarget"
            />
          </AppInfoCard>
          <AppInfoCard
            v-if="documentData.library_path"
            :label="t('document.libraryPath')"
            :value="documentData.library_path"
            :meta="documentData.library_section_label"
          />
          <AppInfoCard :label="t('document.metadata')">
            <pre class="content-pre">{{ formatJson(documentData.metadata_json ?? {}) }}</pre>
          </AppInfoCard>
        </aside>
      </div>
    </AsyncStateBlock>
  </AppPanel>
</template>
