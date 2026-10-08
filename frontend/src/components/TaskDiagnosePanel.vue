<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import {
  apiClient,
  type TaskDiagnoseItem,
  type TaskDiagnoseResponse,
  type TaskItemResponse,
  type TaskItemStatus,
} from "../services/api";
import { queueStageLabel } from "../composables/queue-helpers";
import { summarizeApiError, type ApiErrorSummary } from "../composables/use-error-toast";
import { formatTimestamp } from "../utils/format";
import { libraryDependencyLabel } from "../utils/library-status";

// On-demand operator inspection for one task. The parent expanded row owns the
// item list, pagination, filter, and retry action; this panel only fetches the
// task-scoped diagnose projection when the operator asks for it.
const props = defineProps<{
  taskId: string;
}>();

const { t } = useI18n();

const diagnose = ref<TaskDiagnoseResponse | null>(null);
const isDiagnosing = ref(false);
const diagnoseError = ref<ApiErrorSummary | null>(null);
let diagnoseRequestId = 0;

const diagnoseItems = computed<TaskDiagnoseItem[]>(() =>
  [...(diagnose.value?.items ?? [])].sort((left, right) => left.ordinal - right.ordinal),
);

async function loadDiagnose() {
  const current = ++diagnoseRequestId;
  isDiagnosing.value = true;
  diagnoseError.value = null;
  try {
    const response = await apiClient.getTaskDiagnose(props.taskId);
    if (current !== diagnoseRequestId) return;
    diagnose.value = response;
  } catch (loadError) {
    if (current !== diagnoseRequestId) return;
    diagnose.value = null;
    diagnoseError.value = summarizeApiError(loadError);
  } finally {
    if (current === diagnoseRequestId) isDiagnosing.value = false;
  }
}

function toggleDiagnose() {
  if (diagnose.value || diagnoseError.value) {
    diagnose.value = null;
    diagnoseError.value = null;
    diagnoseRequestId += 1;
    return;
  }
  void loadDiagnose();
}

watch(() => props.taskId, () => {
  diagnose.value = null;
  diagnoseError.value = null;
  diagnoseRequestId += 1;
});

function stageLabel(stage: string | null): string {
  return queueStageLabel(t, stage);
}

function diagnoseWaitingLabel(reason: string | null, dependency: string | null): string {
  if (!reason) return "--";
  const label = t(`processingQueue.waitingReasons.${reason}`);
  return dependency ? `${label}: ${libraryDependencyLabel(t, dependency)}` : label;
}

// One compact fact line per diagnose item: position/ordinal, actual stage,
// attempt count, wait reason or dependency, next retry, lease, active attempt,
// and the latest error. Everything comes from the diagnose projection, so the
// panel explains progress without re-exposing item input.
function diagnoseItemFacts(item: TaskDiagnoseItem): string[] {
  const facts = [
    t("processingQueue.diagnose.position", { position: item.ordinal + 1 }),
    t("processingQueue.diagnose.ordinal", { ordinal: item.ordinal }),
    stageLabel(item.stage ?? null),
    t("processingQueue.attempts", { count: item.attempt_count }),
  ];
  if (item.waiting_reason) {
    facts.push(diagnoseWaitingLabel(item.waiting_reason, item.dependency_key ?? null));
  } else if (item.dependency_key) {
    facts.push(`${t("processingQueue.diagnose.dependency")}: ${libraryDependencyLabel(t, item.dependency_key)}`);
  }
  if (item.next_attempt_at) facts.push(t("processingQueue.diagnose.nextRetry", { time: formatTimestamp(item.next_attempt_at) }));
  if (item.lease_expires_at) facts.push(t("processingQueue.diagnose.leaseExpires", { time: formatTimestamp(item.lease_expires_at) }));
  if (item.active_attempt) facts.push(t("processingQueue.diagnose.activeAttempt", { attempt: item.active_attempt.attempt }));
  if (item.error_message) facts.push(`${t("processingQueue.diagnose.latestError")}: ${item.error_message}`);
  return facts;
}

function itemSeverity(status: TaskItemResponse["status"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}

// Item statuses and dependency-gate states are backend enum identifiers; each
// renders through its locale key so the queue never shows raw English in
// zh-CN. The raw value is kept only as the fallback for a value this build
// does not yet know (a future generated enum member).
const KNOWN_ITEM_STATUSES: ReadonlySet<string> = new Set<TaskItemStatus>([
  "queued",
  "running",
  "waiting",
  "succeeded",
  "failed",
  "cancelled",
]);

function itemStatusLabel(status: TaskItemResponse["status"]): string {
  return KNOWN_ITEM_STATUSES.has(status) ? t(`processingQueue.statuses.${status}`) : status;
}

const KNOWN_GATE_STATES: ReadonlySet<string> = new Set(["open", "half_open", "closed"]);

function gateStateLabel(state: string): string {
  return KNOWN_GATE_STATES.has(state) ? t(`processingQueue.diagnose.gateStates.${state}`) : state;
}
</script>

<template>
  <div class="mb-2">
    <div class="flex flex-wrap items-center gap-2">
      <UButton
        color="neutral"
        variant="outline"
        size="sm"
        icon="i-lucide-stethoscope"
        data-testid="task-diagnose-toggle"
        :loading="isDiagnosing"
        :label="diagnose || diagnoseError ? t('processingQueue.diagnose.hide') : t('processingQueue.diagnose.action')"
        @click="toggleDiagnose"
      />
    </div>
    <div v-if="diagnoseError" class="mt-2 flex flex-wrap items-center gap-2" data-testid="task-diagnose-error">
      <span class="text-sm text-(--ui-error)">
        {{ t("processingQueue.diagnose.failed") }}<span
          v-if="diagnoseError.status != null"
          class="ml-1 font-mono text-xs"
        >· {{ diagnoseError.status }}</span>
      </span>
      <UButton
        color="neutral"
        variant="outline"
        size="sm"
        icon="i-lucide-refresh-cw"
        :loading="isDiagnosing"
        :disabled="isDiagnosing"
        :aria-label="t('processingQueue.diagnose.retry')"
        :title="t('processingQueue.diagnose.retry')"
        @click="loadDiagnose"
      />
    </div>
    <div
      v-else-if="diagnose"
      class="mt-2 rounded-md border border-default p-2 text-xs"
      data-testid="task-diagnose-panel"
    >
      <div class="flex flex-wrap items-center gap-2">
        <UBadge
          :label="diagnose.consistency.consistent ? t('processingQueue.diagnose.consistent') : t('processingQueue.diagnose.inconsistent')"
          :color="diagnose.consistency.consistent ? 'success' : 'error'"
          variant="subtle"
          data-testid="task-diagnose-consistency"
        />
        <span class="text-muted">{{ t("processingQueue.diagnose.observedAt", { time: formatTimestamp(diagnose.observed_at) }) }}</span>
        <span v-if="diagnose.consistency.current_item_id" class="font-mono text-muted" data-testid="task-diagnose-current">
          {{ t("processingQueue.diagnose.currentItem", { item: diagnose.consistency.current_item_id }) }}
        </span>
        <span v-if="diagnose.consistency.open_attempt_count" class="text-muted">
          {{ t("processingQueue.diagnose.openAttempts", { count: diagnose.consistency.open_attempt_count }) }}
        </span>
        <span v-if="diagnose.consistency.near_exhaustion_item_count" class="text-(--ui-warning)">
          {{ t("processingQueue.diagnose.nearExhaustion", { count: diagnose.consistency.near_exhaustion_item_count }) }}
        </span>
      </div>
      <div v-if="!diagnose.consistency.consistent" class="mt-1 text-(--ui-error)" data-testid="task-diagnose-mismatches">
        {{ t("processingQueue.diagnose.mismatches", { fields: diagnose.consistency.mismatches.join(", ") }) }}
      </div>
      <div v-if="diagnose.items_truncated" class="mt-1 text-(--ui-warning)" data-testid="task-diagnose-truncated">
        {{ t("processingQueue.diagnose.truncated", { count: diagnose.items.length }) }}
      </div>
      <div v-if="diagnose.dependency_gates.length" class="mt-1 flex flex-wrap items-center gap-2" data-testid="task-diagnose-gates">
        <span class="text-muted">{{ t("processingQueue.diagnose.dependencyGates") }}</span>
        <span
          v-for="gate in diagnose.dependency_gates"
          :key="gate.dependency_key"
          class="rounded bg-surface-100 px-1.5 py-0.5 font-mono dark:bg-surface-800"
        >
          {{ libraryDependencyLabel(t, gate.dependency_key) }} · {{ gateStateLabel(gate.state) }}<template v-if="gate.next_probe_at"> · {{ t("processingQueue.diagnose.nextProbe", { time: formatTimestamp(gate.next_probe_at) }) }}</template>
        </span>
      </div>
      <div v-if="diagnoseItems.length" class="mt-2 grid gap-1" data-testid="task-diagnose-items">
        <div
          v-for="item in diagnoseItems"
          :key="item.item_id"
          class="flex flex-wrap items-center gap-x-3 gap-y-0.5 rounded bg-surface-50 px-2 py-1 dark:bg-surface-900/40"
          data-testid="diagnose-item"
        >
          <span class="min-w-0 text-muted" data-testid="diagnose-facts">{{ diagnoseItemFacts(item).join(" · ") }}</span>
          <UBadge :label="itemStatusLabel(item.status)" :color="itemSeverity(item.status)" variant="subtle" data-testid="diagnose-status" />
        </div>
      </div>
      <div v-else class="mt-2 text-muted">{{ t("processingQueue.diagnose.noItems") }}</div>
    </div>
  </div>
</template>
