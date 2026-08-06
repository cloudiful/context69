<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
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
const chunkPage = ref(1);
const chunkPageSize = 10;

let controller: AbortController | null = null;

const visibleChunks = computed(() => {
  if (!documentData.value) {
    return [];
  }

  const start = (chunkPage.value - 1) * chunkPageSize;
  return documentData.value.chunks.slice(start, start + chunkPageSize);
});

const libraryRoute = computed(() => {
  if (!documentData.value?.is_library_file || !documentData.value.library_file_id) {
    return null;
  }

  return {
    name: "group-overview",
    params: {
      groupPath: documentData.value.group_path,
    },
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
    chunkPage.value = 1;
    void loadDocument();
  },
);

onBeforeUnmount(() => {
  controller?.abort();
});
</script>

<template>
  <UCard>
    <template #header><h1 class="text-base font-semibold text-color">{{ t("document.title") }}</h1></template>
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
          <UCard>
            <template #header>
              <div class="grid gap-2">
              <div class="flex flex-wrap items-center gap-2">
                <UBadge :label="documentData.source_key" color="neutral" />
                <UBadge :label="documentData.external_id" color="neutral" />
              </div>
                <h2 class="text-lg font-semibold text-color">{{ documentData.title }}</h2>
                <p class="text-sm text-muted-color">{{ formatDate(documentData.published_at) }}</p>
              <p v-if="documentData.library_path" class="break-words">
                {{ documentData.library_path }}
                <span v-if="documentData.library_section_label"> · {{ documentData.library_section_label }}</span>
              </p>
              </div>
            </template>
          </UCard>

          <div class="grid gap-2">
            <UCard
              v-for="chunk in visibleChunks"
              :key="chunk.chunk_id"
            >
              <p class="text-xs font-medium uppercase tracking-[0.08em] text-muted">
                {{ t("document.chunkLabel", { index: chunk.chunk_index }) }}
              </p>
              <pre class="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-elevated px-3 py-2 text-sm leading-6 text-muted">{{ chunk.text }}</pre>
            </UCard>

            <UAlert
              v-if="documentData.chunks.length === 0"
              color="neutral"
              variant="subtle"
              :title="t('document.noBodyChunksTitle')"
              :description="t('document.noBodyChunksMessage')"
            />
          </div>

          <UPagination
            v-if="documentData.chunks.length > chunkPageSize"
            v-model:page="chunkPage"
            :items-per-page="chunkPageSize"
            :total="documentData.chunks.length"
            class="justify-end"
          />
        </div>

        <aside class="grid gap-2">
          <UCard>
            <p class="mb-2 text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("document.published") }}</p>
            <p class="text-sm text-color">{{ formatDate(documentData.published_at) }}</p>
          </UCard>
          <UCard>
            <p class="mb-2 text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("document.updated") }}</p>
            <p class="text-sm text-color">{{ formatTimestamp(documentData.updated_at) }}</p>
          </UCard>
          <UCard>
            <p class="mb-2 text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("document.sourceLink") }}</p>
            <UButton
              color="neutral"
              variant="outline"
              @click="openSourceTarget"
            >
              {{ libraryRoute ? t("document.openLibraryFile") : t("document.openOrigin") }}
            </UButton>
          </UCard>
          <UCard v-if="documentData.library_path">
            <p class="mb-2 text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("document.libraryPath") }}</p>
            <p class="text-sm text-color">{{ documentData.library_path }}</p>
            <p v-if="documentData.library_section_label" class="text-xs text-muted-color">{{ documentData.library_section_label }}</p>
          </UCard>
          <UCard>
            <p class="mb-2 text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("document.metadata") }}</p>
            <pre class="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-lg bg-emphasis px-3 py-2 text-sm leading-6 text-muted-color">{{ formatJson(documentData.metadata_json ?? {}) }}</pre>
          </UCard>
        </aside>
      </div>
    </AsyncStateBlock>
  </UCard>
</template>
