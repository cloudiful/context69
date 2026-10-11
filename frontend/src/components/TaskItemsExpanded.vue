<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import TaskDiagnosePanel from "./TaskDiagnosePanel.vue";
import TaskItemDetail from "./processing-queue/TaskItemDetail.vue";
import { apiClient, type TaskItemResponse, type TaskItemStatus, type TaskResponse } from "../services/api";
import { ACTIVE_STATUSES } from "../composables/queue-helpers";
import { useTaskItemsRefresh, ITEM_REFRESH_INTERVAL_MS } from "../composables/use-task-items-refresh";
import { summarizeApiError, type ApiErrorSummary } from "../composables/use-error-toast";

const props = defineProps<{
  task: TaskResponse;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [task: TaskResponse];
}>();

const { t } = useI18n();

const ITEMS_PAGE_SIZE = 100;

const statusFilter = ref<TaskItemStatus | null>(null);
const items = ref<TaskItemResponse[]>([]);
const nextCursor = ref<string | null>(null);
const isLoading = ref(false);
const isLoadingMore = ref(false);
const error = ref<ApiErrorSummary | null>(null);
let viewRevision = 0;
let isDisposed = false;

type ItemView = { revision: number; taskId: string; status: TaskItemStatus | null };

function captureView(): ItemView {
  return { revision: viewRevision, taskId: props.task.task_id, status: statusFilter.value };
}

function isCurrentView(view: ItemView): boolean {
  return !isDisposed
    && view.revision === viewRevision
    && view.taskId === props.task.task_id
    && view.status === statusFilter.value;
}

function applyItems(response: { items: TaskItemResponse[]; next_cursor?: string | null }) {
  items.value = response.items;
  nextCursor.value = response.next_cursor ?? null;
}

const taskId = computed(() => props.task.task_id);
const isActiveTask = computed(() => ACTIVE_STATUSES.includes(props.task.status));

async function fetchLoadedPages(view: ItemView, minimumItems: number) {
  const refreshed: TaskItemResponse[] = [];
  let cursor: string | null = null;
  do {
    const response = await apiClient.getTaskItems(view.taskId, {
      limit: ITEMS_PAGE_SIZE,
      status: view.status,
      ...(cursor === null ? {} : { cursor }),
    });
    if (!isCurrentView(view)) return;
    refreshed.push(...response.items);
    cursor = response.next_cursor ?? null;
  } while (cursor !== null && refreshed.length < minimumItems);

  if (isCurrentView(view)) {
    applyItems({ items: refreshed, next_cursor: cursor });
    refreshError.value = false;
  }
}

async function reloadLoadedPages() {
  const view = captureView();
  await fetchLoadedPages(view, items.value.length);
}

const {
  refreshError,
  runExclusive,
  scheduleAutomaticRefresh,
  invalidateAutomaticRefresh,
} = useTaskItemsRefresh({
  taskId,
  enabled: isActiveTask,
  intervalMs: ITEM_REFRESH_INTERVAL_MS,
  refresh: reloadLoadedPages,
});

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
  const view = captureView();
  isLoading.value = true;
  error.value = null;
  try {
    await runExclusive(async () => {
      const response = await apiClient.getTaskItems(view.taskId, {
        limit: ITEMS_PAGE_SIZE,
        status: view.status,
      });
      if (!isCurrentView(view)) return;
      applyItems(response);
      refreshError.value = false;
    }, { isCurrent: () => isCurrentView(view) });
  } catch (loadError) {
    if (!isCurrentView(view)) return;
    items.value = [];
    nextCursor.value = null;
    error.value = summarizeApiError(loadError);
  } finally {
    if (isCurrentView(view)) isLoading.value = false;
  }
}

async function loadMore() {
  if (isLoading.value || isLoadingMore.value || nextCursor.value === null) return;
  const view = captureView();
  isLoadingMore.value = true;
  try {
    await runExclusive(async () => {
      const cursor = nextCursor.value;
      if (cursor === null || !isCurrentView(view)) return;
      const response = await apiClient.getTaskItems(view.taskId, {
        limit: ITEMS_PAGE_SIZE,
        cursor,
        status: view.status,
      });
      if (!isCurrentView(view)) return;
      items.value = [...items.value, ...response.items];
      nextCursor.value = response.next_cursor ?? null;
    }, { isCurrent: () => isCurrentView(view) });
  } catch {
    // Retain the cursor and current rows so the user can retry this page.
  } finally {
    if (isCurrentView(view)) isLoadingMore.value = false;
  }
}

function handleFilterChange(value: TaskItemStatus | null) {
  if (statusFilter.value !== value) statusFilter.value = value;
}

function resetView() {
  viewRevision += 1;
  invalidateAutomaticRefresh();
  items.value = [];
  nextCursor.value = null;
  isLoading.value = false;
  isLoadingMore.value = false;
  error.value = null;
  refreshError.value = false;
}

watch(
  [() => props.task.task_id, () => statusFilter.value],
  ([nextTaskId, nextStatus], [previousTaskId]) => {
    resetView();
    if (nextTaskId !== previousTaskId && nextStatus !== null) {
      statusFilter.value = null;
      return;
    }
    void loadFirstPage();
  },
);

watch(
  () => [props.task.task_id, props.task.updated_at] as const,
  ([nextTaskId, nextUpdatedAt], [previousTaskId, previousUpdatedAt]) => {
    if (nextTaskId !== previousTaskId || nextUpdatedAt === previousUpdatedAt) return;
    if (!isActiveTask.value || (items.value.length === 0 && !error.value)) return;
    const view = captureView();
    scheduleAutomaticRefresh(
      () => fetchLoadedPages(view, items.value.length),
      { isCurrent: () => isCurrentView(view) },
    );
  },
);

onBeforeUnmount(() => {
  isDisposed = true;
  viewRevision += 1;
  isLoading.value = false;
  isLoadingMore.value = false;
});

onMounted(() => {
  void loadFirstPage();
});

// Retry is task-scoped: the endpoint requeues every retryable item of the
// parent task, so the emitted value is the task, not the clicked item.
function handleRetry() {
  emit("retry", props.task);
}
</script>

<template>
  <div class="flex flex-col gap-3 p-3">
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
      <!-- A failed periodic reload keeps the last valid list on screen; the
           notice is informational and offers no destructive action. -->
      <span
        v-if="refreshError"
        class="text-xs text-(--ui-warning)"
        data-testid="task-items-refresh-failed"
      >{{ t("processingQueue.itemProgress.refreshFailed") }}</span>
    </div>
    <!-- Troubleshooting is secondary to the progress the operator came for, so
         it sits below the items rather than above them. -->
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
    <div v-else class="flex flex-col gap-1.5">
      <TaskItemDetail
        v-for="item in items"
        :key="item.item_id"
        :item="item"
        :task="props.task"
        :is-acting="props.isActing"
        @retry="handleRetry"
      />
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
    <TaskDiagnosePanel :task-id="props.task.task_id" />
  </div>
</template>
