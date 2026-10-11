<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { TaskItemResponse } from "../../services/api";
import { formatTimestamp } from "../../utils/format";

import {
  itemProgressView,
  progressCountText,
  progressPercent,
  type TaskItemProgress,
} from "./task-item-progress";

// One item's observed progress. Everything rendered here is an observation the
// worker reported: a named stage, exact counters when both ends are known, the
// last time real progress was seen, and the remote converter status while a
// Docling conversion runs.
//
// The component deliberately renders no overall percentage, no page fraction
// and no stall claim. A counter the server did not report stays absent rather
// than becoming a zero, and "last progress" is an observation, not proof that
// anything is stuck.
const props = defineProps<{
  progress?: TaskItemResponse["progress"];
}>();

const { t } = useI18n();

const view = computed(() => itemProgressView(t, props.progress));

const chunkPercent = computed(() => progressPercent(view.value?.chunks ?? null));
const batchPercent = computed(() => progressPercent(view.value?.batches ?? null));
const chunkText = computed(() => progressCountText(t, view.value?.chunks ?? null, "chunks"));
const batchText = computed(() => progressCountText(t, view.value?.batches ?? null, "batches"));
</script>

<template>
  <div
    v-if="view"
    class="flex min-w-0 flex-col gap-1.5"
    data-testid="task-item-progress"
  >
    <div class="flex flex-wrap items-center gap-2">
      <UBadge
        :label="view.stageLabel"
        color="neutral"
        variant="subtle"
        data-testid="item-progress-stage"
      />
      <span
        v-if="view.isComplete"
        class="text-xs text-(--ui-success)"
        data-testid="item-progress-complete"
      >{{ t("processingQueue.itemProgress.complete") }}</span>
      <span
        v-else-if="view.isUnknown"
        class="text-xs text-muted"
        data-testid="item-progress-unknown"
      >{{ t("processingQueue.itemProgress.countUnknown") }}</span>
    </div>

    <!-- Chunk and batch counters are separate units and stay separate rows:
         a batch is a write of up to 32 chunks, so reading one as the other
         would overstate or understate real work by that factor. -->
    <div
      v-if="chunkText || batchText"
      class="flex flex-col gap-1"
      data-testid="item-progress-counts"
    >
      <div
        v-if="chunkText"
        class="flex flex-wrap items-center gap-2"
        data-testid="item-progress-chunks"
      >
        <span class="text-xs text-muted">{{ t("processingQueue.itemProgress.chunks") }}</span>
        <span class="text-xs text-(--ui-text)">{{ chunkText }}</span>
        <span
          v-if="chunkPercent !== null"
          class="h-1 w-24 min-w-0 flex-1 overflow-hidden rounded-full bg-surface-200 dark:bg-surface-800"
          data-testid="item-progress-bar"
        >
          <span
            class="block h-full rounded-full bg-(--ui-primary)"
            :style="{ width: `${chunkPercent}%` }"
          />
        </span>
      </div>
      <div
        v-if="batchText"
        class="flex flex-wrap items-center gap-2"
        data-testid="item-progress-batches"
      >
        <span class="text-xs text-muted">{{ t("processingQueue.itemProgress.batches") }}</span>
        <span class="text-xs text-(--ui-text)">{{ batchText }}</span>
        <span
          v-if="batchPercent !== null"
          class="h-1 w-24 min-w-0 flex-1 overflow-hidden rounded-full bg-surface-200 dark:bg-surface-800"
          data-testid="item-progress-bar"
        >
          <span
            class="block h-full rounded-full bg-(--ui-primary)"
            :style="{ width: `${batchPercent}%` }"
          />
        </span>
      </div>
    </div>

    <!-- Docling Serve reports a task status and no page counts, so the UI says
         so explicitly instead of implying a page fraction it does not have.
         This copy is scoped to the Docling stage: text ingestion has no remote
         conversion and must not imply one. -->
    <p
      v-if="view.showsPageProgressUnavailable"
      class="text-xs text-muted"
      data-testid="item-progress-page-unavailable"
    >
      {{ t("processingQueue.itemProgress.pageProgressUnavailable") }}
      <span
        v-if="view.remoteStatus"
        class="ml-1"
        data-testid="item-progress-remote-status"
      >{{ t("processingQueue.itemProgress.remoteStatus", { status: view.remoteStatus }) }}</span>
    </p>

    <span
      v-if="view.lastProgressAt"
      class="text-xs text-muted"
      data-testid="item-progress-observed"
    >{{ t("processingQueue.itemProgress.lastObserved", { time: formatTimestamp(view.lastProgressAt) }) }}</span>
  </div>
</template>