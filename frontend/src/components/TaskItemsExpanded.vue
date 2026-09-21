<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import TaskItemAction from "./TaskItemAction.vue";
import { apiClient, type TaskItemResponse, type TaskItemStatus, type TaskResponse } from "../services/api";
import { summarizeApiError, type ApiErrorSummary } from "../composables/use-error-toast";

const props = defineProps<{
  task: TaskResponse;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [task: TaskResponse];
}>();

const { t } = useI18n();

// Page size matches the previous single-fetch limit so existing callers and
// tests asserting `{ limit: 100 }` stay valid. Follow `next_cursor` until null
// to walk the full filtered set (2781-scale); the backend pins active-first
// order (failed/running first, succeeded last) so no client sort is needed.
const ITEMS_PAGE_SIZE = 100;
const ACTIVE_ITEM_REFRESH_STATUSES: TaskResponse["status"][] = ["queued", "running", "waiting"];

const statusFilter = ref<TaskItemStatus | null>(null);
const items = ref<TaskItemResponse[]>([]);
const nextCursor = ref<string | null>(null);
const isLoading = ref(false);
const isLoadingMore = ref(false);
const error = ref<ApiErrorSummary | null>(null);
let requestId = 0;

const hasMore = computed(() => nextCursor.value != null);
const shownCount = computed(() => items.value.length);
const totalCount = computed(() => props.task.progress.total);

const statusOptions = computed(() => [
  { label: t("processingQueue.allItemStatuses"), value: null },
  ...(["queued", "running", "waiting", "succeeded", "failed", "cancelled"] as TaskItemStatus[]).map((value) => ({
    label: t(`processingQueue.statuses.${value}`),
    value,
  })),
]);

async function loadFirstPage() {
  const current = ++requestId;
  isLoading.value = true;
  error.value = null;
  try {
    const response = await apiClient.getTaskItems(props.task.task_id, {
      limit: ITEMS_PAGE_SIZE,
      status: statusFilter.value,
    });
    if (current !== requestId) return;
    items.value = response.items;
    nextCursor.value = response.next_cursor ?? null;
  } catch (loadError) {
    if (current !== requestId) return;
    items.value = [];
    nextCursor.value = null;
    error.value = summarizeApiError(loadError);
  } finally {
    if (current === requestId) isLoading.value = false;
  }
}

async function loadMore() {
  if (isLoading.value || isLoadingMore.value || nextCursor.value == null) return;
  const current = ++requestId;
  isLoadingMore.value = true;
  try {
    const response = await apiClient.getTaskItems(props.task.task_id, {
      limit: ITEMS_PAGE_SIZE,
      cursor: nextCursor.value,
      status: statusFilter.value,
    });
    if (current !== requestId) return;
    items.value = [...items.value, ...response.items];
    nextCursor.value = response.next_cursor ?? null;
  } catch {
    // Keep the cursor so the user can retry: a failed page appends nothing.
  } finally {
    if (current === requestId) isLoadingMore.value = false;
  }
}

function handleFilterChange(value: TaskItemStatus | null) {
  if (statusFilter.value === value) return;
  // Cursor is scoped to the filter: reset to no cursor when it changes.
  statusFilter.value = value;
}

watch(statusFilter, () => {
  nextCursor.value = null;
  void loadFirstPage();
});

watch(() => props.task.task_id, () => {
  statusFilter.value = null;
  items.value = [];
  nextCursor.value = null;
  void loadFirstPage();
});

// Live refresh for active tasks: the queue SSE stream (3s per-task coalesced
// server-side) bumps `task.updated_at` via the in-place merge. Reset to the
// first page so the active-first pinning stays fresh; the user can page again
// for the remainder. Inactive tasks never auto-refresh.
watch(() => props.task.updated_at, (next, prev) => {
  if (next === prev) return;
  if (!ACTIVE_ITEM_REFRESH_STATUSES.includes(props.task.status)) return;
  if (isLoading.value || isLoadingMore.value) return;
  if (items.value.length === 0 && !error.value) return;
  void loadFirstPage();
});

onMounted(() => {
  void loadFirstPage();
});

function stageLabel(stage: string | null): string {
  return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage");
}

function itemSeverity(status: TaskItemResponse["status"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}
</script>

<template>
  <div class="p-3">
    <div class="mb-2 flex flex-wrap items-center gap-2">
      <span class="text-xs text-muted" data-testid="task-items-count">
        {{ t("processingQueue.itemsCount", { shown: shownCount, total: totalCount }) }}
      </span>
      <USelect
        :model-value="statusFilter"
        :items="statusOptions"
        value-key="value"
        size="sm"
        class="w-36"
        data-testid="task-items-filter"
        :aria-label="t('processingQueue.itemStatusFilter')"
        @update:model-value="handleFilterChange($event as TaskItemStatus | null)"
      />
    </div>
    <template v-if="isLoading">
      <div class="text-sm text-muted">{{ t("common.loading") }}…</div>
    </template>
    <template v-else-if="error && items.length === 0">
      <div class="flex flex-wrap items-center gap-2">
        <span
          class="text-sm text-(--ui-error)"
          :title="error.message || undefined"
          :aria-label="t('processingQueue.itemsLoadFailed')"
        >
          {{ t("processingQueue.itemsLoadFailed") }}<span
            v-if="error.status != null"
            class="ml-1 font-mono text-xs"
          >· {{ error.status }}</span>
        </span>
        <UButton
          color="neutral"
          variant="outline"
          size="sm"
          icon="i-lucide-refresh-cw"
          :loading="isLoading"
          :disabled="isLoading"
          :aria-label="t('processingQueue.retryLoadItems')"
          :title="t('processingQueue.retryLoadItems')"
          @click="loadFirstPage"
        />
      </div>
    </template>
    <template v-else-if="items.length === 0">
      <div class="text-sm text-muted">{{ t("processingQueue.noItems") }}</div>
    </template>
    <div v-else class="grid gap-1">
      <div
        v-for="item in items"
        :key="item.item_id"
        class="grid grid-cols-[minmax(0,1fr)_auto_auto_minmax(0,1fr)_auto_auto] items-center gap-3 rounded-md bg-surface-50 dark:bg-surface-900/40 px-3 py-1.5 text-sm"
      >
        <span class="block truncate font-mono text-xs text-muted" :title="item.item_id">{{ item.item_id }}</span>
        <UBadge :label="item.status" :color="itemSeverity(item.status)" variant="subtle" />
        <span class="whitespace-nowrap text-xs text-muted">{{ stageLabel(item.stage ?? null) }}</span>
        <span class="block truncate text-xs text-muted" :title="item.error_message || undefined">{{ item.error_message || "--" }}</span>
        <span class="whitespace-nowrap text-xs text-muted">{{ t("processingQueue.attempts", { count: item.attempt_count }) }}</span>
        <TaskItemAction
          :item="item"
          :task="props.task"
          :is-acting="props.isActing"
          @retry="emit('retry', $event)"
        />
      </div>
      <div v-if="hasMore" class="mt-1 flex justify-center">
        <UButton
          color="neutral"
          variant="outline"
          size="sm"
          icon="i-lucide-chevron-down"
          data-testid="task-items-load-more"
          :loading="isLoadingMore"
          :disabled="isLoadingMore"
          :label="t('processingQueue.loadMoreItems')"
          @click="loadMore"
        />
      </div>
    </div>
  </div>
</template>
