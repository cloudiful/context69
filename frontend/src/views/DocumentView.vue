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
import { apiClient, ApiError, type DocumentResponse } from "../services/api";
import { formatDate, formatJson, formatTimestamp } from "../utils/format";
import { useErrorToast } from "../composables/use-error-toast";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();

const documentData = ref<DocumentResponse | null>(null);
const loadError = ref<string | null>(null);
const loading = ref(false);
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
    documentData.value = null;
    loadError.value = t("document.invalidId");
    showErrorToast(null, t("document.invalidId"));
    return;
  }

  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  loadError.value = null;

  try {
    documentData.value = await apiClient.getDocument(documentId, {
      signal: controller.signal,
    });
  } catch (error) {
    documentData.value = null;
    loadError.value = error instanceof ApiError
      ? `${error.status}: ${error.message}`
      : t("document.loadFailed");
    showErrorToast(error, t("document.loadFailed"));
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
      :error="loadError"
      :error-title="t('document.loadFailed')"
      loading-test-id="document-loading"
      :loading-title="t('common.loading')"
      :loading-message="t('document.loadingMessage')"
    >
      <div v-if="documentData" class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_20rem]">
        <div class="grid gap-2">
          <AppRecordCard
            :title="documentData.title"
            :subtitle="formatDate(documentData.published_at)"
          >
            <template #tags>
              <div class="flex flex-wrap items-center gap-2">
                <Tag :value="documentData.source_key" severity="secondary" />
                <Tag :value="documentData.external_id" severity="secondary" />
              </div>
            </template>

            <template #meta>
              <p v-if="documentData.library_path" class="break-words">
                {{ documentData.library_path }}
                <span v-if="documentData.library_section_label"> · {{ documentData.library_section_label }}</span>
              </p>
            </template>
          </AppRecordCard>

          <div class="grid gap-2">
            <Card
              v-for="chunk in visibleChunks"
              :key="chunk.chunk_id"
            >
              <template #content>
                <p class="text-xs font-medium uppercase tracking-[0.08em] text-muted-color">
                  {{ t("document.chunkLabel", { index: chunk.chunk_index }) }}
                </p>
                <pre class="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-lg bg-emphasis px-3 py-2 text-sm leading-6 text-muted-color">{{ chunk.text }}</pre>
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
            severity="secondary"
            variant="outlined"
            type="button"
            @click="expanded = !expanded"
          >
            {{ expanded ? t("document.collapse") : t("document.showAll", { count: documentData.chunks.length }) }}
          </Button>
        </div>

        <aside class="grid gap-2">
          <AppInfoCard :label="t('document.published')" :value="formatDate(documentData.published_at)" />
          <AppInfoCard :label="t('document.updated')" :value="formatTimestamp(documentData.updated_at)" />
          <AppInfoCard :label="t('document.sourceLink')">
            <Button
              severity="secondary"
              variant="outlined"
              @click="openSourceTarget"
            >
              {{ libraryRoute ? t("document.openLibraryFile") : t("document.openOrigin") }}
            </Button>
          </AppInfoCard>
          <AppInfoCard
            v-if="documentData.library_path"
            :label="t('document.libraryPath')"
            :value="documentData.library_path"
            :meta="documentData.library_section_label"
          />
          <AppInfoCard :label="t('document.metadata')">
            <pre class="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-lg bg-emphasis px-3 py-2 text-sm leading-6 text-muted-color">{{ formatJson(documentData.metadata_json ?? {}) }}</pre>
          </AppInfoCard>
        </aside>
      </div>
    </AsyncStateBlock>
  </AppPanel>
</template>
