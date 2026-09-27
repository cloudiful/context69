<script setup lang="ts">
import { useI18n } from "vue-i18n";
import type { DropdownMenuItem } from "@nuxt/ui";

const props = defineProps<{
  createMenuItems: DropdownMenuItem[];
  deleteSourceAfterProcessing: boolean;
  query: string;
  retryAllBusy: boolean;
  retryAllFailedCount: number;
  scopedQuery: string;
  scopedSearchLoading: boolean;
  scopedSearchPlaceholder: string;
  uploadBusy: boolean;
}>();

const emit = defineEmits<{
  "retryAllFailed": [];
  "runScopedSearch": [];
  "update:deleteSourceAfterProcessing": [value: boolean];
  "update:query": [value: string];
  "update:scopedQuery": [value: string];
  "upload": [];
}>();

const { t } = useI18n();
</script>

<template>
  <Teleport to="#app-route-actions">
    <div class="flex items-center gap-2">
      <UInput
        :model-value="props.query"
        class="w-40 sm:w-56"
        icon="i-lucide-search"
        :placeholder="t('nav.search')"
        @update:model-value="emit('update:query', String($event ?? ''))"
      />
      <UInput
        :model-value="props.scopedQuery"
        class="hidden w-40 sm:inline-flex md:w-56"
        icon="i-lucide-scan-text"
        data-testid="scoped-content-search-input"
        :aria-label="props.scopedSearchPlaceholder"
        :placeholder="props.scopedSearchPlaceholder"
        :title="props.scopedSearchPlaceholder"
        @update:model-value="emit('update:scopedQuery', String($event ?? ''))"
        @keydown.enter="emit('runScopedSearch')"
      />
      <UButton
        class="hidden shrink-0 sm:inline-flex"
        icon="i-lucide-scan-text"
        data-testid="scoped-content-search-trigger"
        :label="t('search.scoped.run')"
        :loading="props.scopedSearchLoading"
        @click="emit('runScopedSearch')"
      />
      <UButton
        icon="i-lucide-rotate-ccw"
        :label="t('library.retryAllFailed')"
        class="hidden shrink-0 sm:inline-flex"
        data-testid="retry-all-failed"
        :loading="props.retryAllBusy"
        :disabled="props.retryAllBusy || props.retryAllFailedCount === 0"
        :title="t('library.retryAllFailed')"
        @click="emit('retryAllFailed')"
      />
      <UButton
        icon="i-lucide-rotate-ccw"
        aria-label="Retry all failed"
        class="shrink-0 sm:hidden"
        data-testid="retry-all-failed-compact"
        :loading="props.retryAllBusy"
        :disabled="props.retryAllBusy || props.retryAllFailedCount === 0"
        @click="emit('retryAllFailed')"
      />
      <UDropdownMenu :items="props.createMenuItems" :content="{ align: 'end' }">
        <UButton icon="i-lucide-plus" :label="t('common.new')" class="hidden sm:inline-flex" />
        <UButton icon="i-lucide-plus" aria-label="New" class="sm:hidden" />
      </UDropdownMenu>
      <UCheckbox
        :model-value="props.deleteSourceAfterProcessing"
        binary
        class="hidden md:flex"
        :label="t('library.releaseSourceAfterUpload')"
        :title="t('library.releaseSourceAfterUploadHint')"
        :aria-label="t('library.releaseSourceAfterUpload')"
        @update:model-value="emit('update:deleteSourceAfterProcessing', $event === true)"
      />
      <UButton
        icon="i-lucide-upload"
        :label="t('common.upload')"
        class="hidden sm:inline-flex"
        :loading="props.uploadBusy"
        @click="emit('upload')"
      />
      <UButton
        icon="i-lucide-upload"
        aria-label="Upload"
        class="sm:hidden"
        :loading="props.uploadBusy"
        @click="emit('upload')"
      />
    </div>
  </Teleport>
</template>
