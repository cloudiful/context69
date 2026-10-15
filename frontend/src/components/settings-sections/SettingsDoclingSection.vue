<script setup lang="ts">
import { computed } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";
import UTooltip from "@nuxt/ui/components/Tooltip.vue";

import AppSelectField from "../AppSelectField.vue";
import AppSettingsBlock from "../AppSettingsBlock.vue";
import AppSettingsSection from "../AppSettingsSection.vue";
import AppTextField from "../AppTextField.vue";
import AppNumberField from "../AppNumberField.vue";
import SettingsTestButton from "./SettingsTestButton.vue";
import type { DraftDoclingSettings, DraftDoclingVlmMode } from "../../utils/settings";
import type { DoclingRemoteTask, DoclingCleanupTask } from "../../services/api/api-types";

const props = withDefaults(
  defineProps<{
    doclingDraft: DraftDoclingSettings;
    doclingTesting: boolean;
    remoteItems?: DoclingRemoteTask[];
    cleanupItems?: DoclingCleanupTask[];
    cleanupWoken?: number | null;
    cleaning?: boolean;
    remoteLoading?: boolean;
    remoteError?: string | null;
    remoteErrorStatus?: number | null;
    remoteHasMore?: boolean;
    remoteOffset?: number;
    remoteLoadedOnce?: boolean;
  }>(),
  {
    remoteItems: () => [],
    cleanupItems: () => [],
    cleanupWoken: null,
    cleaning: false,
    remoteLoading: false,
    remoteError: null,
    remoteErrorStatus: null,
    remoteHasMore: false,
    remoteOffset: 0,
    remoteLoadedOnce: false,
  },
);

const emit = defineEmits<{
  "test-docling": [];
  "clean-stopped": [];
  "refresh-remote": [];
  "prev-remote": [];
  "next-remote": [];
}>();

const { t } = useI18n();

const vlmModeOptions = computed(() => [
  { label: t("settings.docling.vlmModeDisabled"), value: "disabled" },
  { label: t("settings.docling.vlmModePreset"), value: "preset" },
  { label: t("settings.docling.vlmModeCustom"), value: "custom" },
]);

const vlmModeInfo = computed(() => {
  const mode: DraftDoclingVlmMode = props.doclingDraft.vlm_mode;
  switch (mode) {
    case "disabled":
      return t("settings.docling.vlmModeInfoDisabled");
    case "preset":
      return t("settings.docling.vlmModeInfoPreset");
    case "custom":
      return t("settings.docling.vlmModeInfoCustom");
  }
});

const vlmModeInfoLabel = computed(() => t("settings.docling.vlmModeInfoLabel"));

const remoteColumns = computed<TableColumn<DoclingRemoteTask>[]>(() => [
  { accessorKey: "task_id", header: t("settings.docling.remoteTasks.taskId") },
  { accessorKey: "task_type", header: t("settings.docling.remoteTasks.taskType") },
  { id: "task_status", header: t("settings.docling.remoteTasks.taskStatus") },
  { accessorKey: "created_at", header: t("settings.docling.remoteTasks.createdAt") },
]);

const cleanupColumns = computed<TableColumn<DoclingCleanupTask>[]>(() => [
  { accessorKey: "remote_task_id", header: t("settings.docling.remoteTasks.taskId") },
  { id: "status", header: t("settings.docling.remoteTasks.taskStatus") },
  { id: "outcome", header: t("settings.docling.remoteTasks.outcome") },
  { id: "reason_code", header: t("settings.docling.remoteTasks.reason") },
]);

type CleanupReasonCode = NonNullable<DoclingCleanupTask["reason_code"]>;

function cleanupStatusLabel(status: string): string {
  switch (status) {
    case "pending": return t("settings.docling.remoteTasks.statusPending");
    case "cancelling": return t("settings.docling.remoteTasks.statusCancelling");
    case "awaiting_terminal": return t("settings.docling.remoteTasks.statusAwaitingTerminal");
    case "done": return t("settings.docling.remoteTasks.statusDone");
    case "unsupported": return t("settings.docling.remoteTasks.statusUnsupported");
    case "failed": return t("settings.docling.remoteTasks.statusFailed");
    default: return t("settings.docling.remoteTasks.statusUnknown");
  }
}

function cleanupStatusColor(status: string): "success" | "info" | "warning" | "error" | "neutral" {
  switch (status) {
    case "done": return "success";
    case "failed": return "error";
    case "pending": return "warning";
    case "cancelling":
    case "awaiting_terminal": return "info";
    default: return "neutral";
  }
}

function cleanupOutcomeLabel(outcome: string | null | undefined): string {
  switch (outcome) {
    case "success": return t("settings.docling.remoteTasks.outcomeSuccess");
    case "failure": return t("settings.docling.remoteTasks.outcomeFailure");
    case "cancelled": return t("settings.docling.remoteTasks.outcomeCancelled");
    case "not_found": return t("settings.docling.remoteTasks.outcomeNotFound");
    case null:
    case undefined:
    case "": return t("settings.docling.remoteTasks.outcomeUnknown");
    default: return t("settings.docling.remoteTasks.outcomeUnknown");
  }
}

function cleanupReasonLabel(reason: CleanupReasonCode | null | undefined): string {
  switch (reason) {
    case "missing_submit_endpoint": return t("settings.docling.remoteTasks.reasonMissingSubmitEndpoint");
    case "authentication_failed": return t("settings.docling.remoteTasks.reasonAuthenticationFailed");
    case "deadline_exceeded": return t("settings.docling.remoteTasks.reasonDeadlineExceeded");
    case "remote_not_found": return t("settings.docling.remoteTasks.reasonRemoteNotFound");
    case "awaiting_remote_terminal": return t("settings.docling.remoteTasks.reasonAwaitingRemoteTerminal");
    case "unspecified":
    case null:
    case undefined:
    default: return t("settings.docling.remoteTasks.reasonUnspecified");
  }
}

function cleanupReasonDetail(reason: CleanupReasonCode | null | undefined): string {
  switch (reason) {
    case "missing_submit_endpoint": return t("settings.docling.remoteTasks.reasonMissingSubmitEndpointDetail");
    case "authentication_failed": return t("settings.docling.remoteTasks.reasonAuthenticationFailedDetail");
    case "deadline_exceeded": return t("settings.docling.remoteTasks.reasonDeadlineExceededDetail");
    case "remote_not_found": return t("settings.docling.remoteTasks.reasonRemoteNotFoundDetail");
    case "awaiting_remote_terminal": return t("settings.docling.remoteTasks.reasonAwaitingRemoteTerminalDetail");
    case "unspecified":
    case null:
    case undefined:
    default: return t("settings.docling.remoteTasks.reasonUnspecifiedDetail");
  }
}

const remoteDenied = computed(() => props.remoteErrorStatus === 403);
const showRemoteLoading = computed(() => props.remoteLoading && !props.remoteLoadedOnce);
const showRemoteEmpty = computed(
  () => !props.remoteLoading && !props.remoteError && props.remoteLoadedOnce && props.remoteItems.length === 0,
);
const canPrev = computed(() => props.remoteOffset > 0 && !props.remoteLoading);
const canNext = computed(() => props.remoteHasMore && !props.remoteLoading);
</script>

<template>
  <AppSettingsSection :legend="t('settings.docling.title')">
    <div class="grid gap-6">
      <AppSettingsBlock id="settings-connection" compact :title="t('settings.docling.connectionTitle')">
        <template #actions>
          <SettingsTestButton
            :testing="doclingTesting"
            :disabled="!doclingDraft.connection.base_url.trim()"
            test-id="docling-connection-test"
            :label="t('settings.docling.test')"
            @test="emit('test-docling')"
          />
        </template>
        <div class="grid max-w-2xl gap-3">
          <AppTextField
            input-id="docling-base-url"
            v-model="doclingDraft.connection.base_url"
            name="docling.connection.base_url"
            :label="t('settings.docling.baseUrl')"
            type="url"
            placeholder="http://127.0.0.1:5001"
          />
          <AppNumberField
            input-id="docling-timeout"
            v-model="doclingDraft.connection.timeout_secs"
            name="docling.connection.timeout_secs"
            :label="t('settings.docling.timeout')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="docling-poll-interval"
            v-model="doclingDraft.connection.poll_interval_secs"
            name="docling.connection.poll_interval_secs"
            :label="t('settings.docling.pollInterval')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="docling-task-timeout"
            v-model="doclingDraft.connection.task_timeout_secs"
            name="docling.connection.task_timeout_secs"
            :label="t('settings.docling.taskTimeout')"
            :min="1"
            :step="1"
          />
          <AppNumberField
            input-id="docling-max-inflight"
            v-model="doclingDraft.connection.max_inflight"
            name="docling.connection.max_inflight"
            :label="t('settings.docling.maxInflight')"
            :helper="t('settings.docling.maxInflightHelper')"
            :min="1"
            :max="32"
            :step="1"
          />
        </div>
      </AppSettingsBlock>

      <AppSettingsBlock id="settings-vlm" compact :title="t('settings.docling.vlmTitle')">
        <div class="grid gap-3">
          <div class="grid max-w-2xl gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:items-end">
            <AppSelectField
              input-id="docling-vlm-mode"
              v-model="doclingDraft.vlm_mode"
              name="docling.vlm_mode"
              :label="t('settings.docling.vlmModeLabel')"
              :options="vlmModeOptions"
            />
            <UTooltip :text="vlmModeInfo" :ui="{ content: 'max-w-xs text-xs leading-5' }">
              <UButton
                data-testid="docling-vlm-mode-info"
                icon="i-lucide-info"
                color="neutral"
                variant="ghost"
                size="sm"
                :aria-label="vlmModeInfoLabel"
                :title="vlmModeInfoLabel"
              />
            </UTooltip>
          </div>

          <template v-if="doclingDraft.vlm_mode === 'preset'">
            <div class="grid max-w-2xl gap-3">
              <AppTextField
                input-id="docling-picture-description-preset"
                v-model="doclingDraft.vlm.picture_description_preset"
                name="docling.vlm.picture_description_preset"
                :label="t('settings.docling.pictureDescriptionPreset')"
                :placeholder="t('settings.docling.pictureDescriptionPresetPlaceholder')"
              />
            </div>
          </template>

          <template v-else-if="doclingDraft.vlm_mode === 'custom'">
            <div class="grid max-w-2xl gap-3">
              <AppTextField
                input-id="docling-openai-base-url"
                v-model="doclingDraft.vlm.openai_base_url"
                name="docling.vlm.openai_base_url"
                :label="t('settings.docling.openAiBaseUrl')"
                type="url"
                placeholder="https://openrouter.ai/api/v1"
              />
            </div>

            <div class="grid max-w-2xl gap-3">
              <AppTextField
                input-id="docling-api-key"
                v-model="doclingDraft.vlm.api_key"
                name="docling.vlm.api_key"
                :label="t('settings.docling.apiKey')"
                type="password"
                autocomplete="new-password"
                placeholder="sk-..."
              />
              <AppTextField
                input-id="docling-vlm-pipeline-model"
                v-model="doclingDraft.vlm.vlm_pipeline_model"
                name="docling.vlm.vlm_pipeline_model"
                :label="t('settings.docling.vlmPipelineModel')"
                placeholder="gemini-3-flash"
              />
            </div>

            <div class="grid max-w-2xl gap-3">
              <AppTextField
                input-id="docling-picture-description-model"
                v-model="doclingDraft.vlm.picture_description_model"
                name="docling.vlm.picture_description_model"
                :label="t('settings.docling.pictureDescriptionModel')"
                placeholder="gpt-4o-mini"
              />
              <AppTextField
                input-id="docling-code-formula-model"
                v-model="doclingDraft.vlm.code_formula_model"
                name="docling.vlm.code_formula_model"
                :label="t('settings.docling.codeFormulaModel')"
                placeholder="gpt-4o-mini"
              />
            </div>
          </template>
        </div>
      </AppSettingsBlock>

      <AppSettingsBlock
        id="settings-docling-remote-tasks"
        compact
        :title="t('settings.docling.remoteTasks.title')"
        :description="t('settings.docling.remoteTasks.description')"
      >
        <template #actions>
          <UButton
            data-testid="docling-remote-refresh"
            color="neutral"
            variant="outline"
            size="sm"
            icon="i-lucide-refresh-cw"
            :loading="remoteLoading"
            :disabled="remoteLoading"
            @click="emit('refresh-remote')"
          >
            {{ t("settings.docling.remoteTasks.refresh") }}
          </UButton>
        </template>

        <div v-if="showRemoteLoading" data-testid="docling-remote-loading" class="text-sm text-muted">
          {{ t("settings.docling.remoteTasks.loading") }}
        </div>

        <div
          v-else-if="remoteError"
          data-testid="docling-remote-error"
          class="grid gap-1 text-sm"
          role="alert"
        >
          <span v-if="remoteDenied">{{ t("settings.docling.remoteTasks.denied") }}</span>
          <span v-else>{{ t("settings.docling.remoteTasks.loadFailed") }}: {{ remoteError }}</span>
          <span v-if="remoteErrorStatus !== null" class="text-xs text-muted">
            {{ remoteErrorStatus }}
          </span>
        </div>

        <div v-else-if="showRemoteEmpty" data-testid="docling-remote-empty" class="text-sm text-muted">
          {{ t("settings.docling.remoteTasks.empty") }}
        </div>

        <template v-else-if="remoteLoadedOnce">
          <UTable
            class="min-w-0 max-w-full"
            :ui="{ th: 'whitespace-nowrap' }"
            :data="remoteItems"
            :columns="remoteColumns"
            :loading="remoteLoading"
            :empty="t('settings.docling.remoteTasks.empty')"
          >
            <template #task_status-cell="{ row }">
              <UBadge
                class="whitespace-nowrap"
                :label="row.original.task_status"
                color="neutral"
                variant="subtle"
              />
            </template>
          </UTable>

          <div class="flex items-center gap-2 pt-2">
            <UButton
              data-testid="docling-remote-prev"
              color="neutral"
              variant="outline"
              size="sm"
              :disabled="!canPrev"
              @click="emit('prev-remote')"
            >
              {{ t("settings.docling.remoteTasks.previous") }}
            </UButton>
            <UButton
              data-testid="docling-remote-next"
              color="neutral"
              variant="outline"
              size="sm"
              :disabled="!canNext"
              @click="emit('next-remote')"
            >
              {{ t("settings.docling.remoteTasks.next") }}
            </UButton>
          </div>
        </template>

        <div v-else data-testid="docling-remote-empty" class="text-sm text-muted">
          {{ t("settings.docling.remoteTasks.empty") }}
        </div>
      </AppSettingsBlock>
      <AppSettingsBlock :title="t('settings.docling.remoteTasks.cleanupTitle')" :description="t('settings.docling.remoteTasks.cleanupDescription')">
        <div class="grid gap-2">
          <div>
            <UButton :loading="cleaning" :disabled="remoteDenied || remoteLoading || cleaning" @click="emit('clean-stopped')">
              {{ t('settings.docling.remoteTasks.cleanStopped') }}
            </UButton>
          </div>
          <p class="text-xs text-muted">{{ t("settings.docling.remoteTasks.cleanupEligibility") }}</p>
          <p v-if="cleanupWoken !== null" data-testid="docling-cleanup-woken" class="text-sm">
            {{ t("settings.docling.remoteTasks.cleanupWoken", { count: cleanupWoken }) }}
          </p>
        </div>
        <UTable :data="cleanupItems" :columns="cleanupColumns" :empty="t('settings.docling.remoteTasks.cleanupEmpty')">
          <template #status-cell="{ row }">
            <UBadge
              class="whitespace-nowrap"
              :label="cleanupStatusLabel(row.original.status)"
              :color="cleanupStatusColor(row.original.status)"
              variant="subtle"
            />
          </template>
          <template #outcome-cell="{ row }">
            <span>{{ cleanupOutcomeLabel(row.original.outcome) }}</span>
          </template>
          <template #reason_code-cell="{ row }">
            <span class="grid gap-0.5">
              <span>{{ cleanupReasonLabel(row.original.reason_code) }}</span>
              <span class="text-xs text-muted">{{ cleanupReasonDetail(row.original.reason_code) }}</span>
            </span>
          </template>
        </UTable>
      </AppSettingsBlock>
    </div>
  </AppSettingsSection>
</template>
