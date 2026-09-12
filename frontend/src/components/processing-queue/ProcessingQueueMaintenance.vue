<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import AppNumberField from "../AppNumberField.vue";
import TaskQuarantineDialog from "../TaskQuarantineDialog.vue";
import type {
  QuarantineStaleSubmittingResponse,
  TaskMaintenanceSettings,
  TaskMaintenanceStats,
  TaskPurgeMode,
} from "../../services/api";

const props = defineProps<{
  error: string | null;
  stats: TaskMaintenanceStats | null;
  activeCount: number;
  uncertainSubmitting: number | null;
  quarantinableSubmitting: number | null;
  orphanedExternalJobs: number | null;
  lastQuarantine: QuarantineStaleSubmittingResponse | null;
  action: "cancel" | "purge" | "quarantine" | null;
  saving: boolean;
  settings: TaskMaintenanceSettings | null;
}>();

const emit = defineEmits<{
  confirmCancel: [];
  confirmPurge: [mode: TaskPurgeMode];
  saveSettings: [cleanupEnabled: boolean, retentionDays: number];
  quarantine: [reason: string, grace: number, limit: number];
}>();

const { t } = useI18n();

const draftCleanup = ref(true);
const draftRetentionDays = ref(30);
const settingsOpen = ref(false);
const quarantineOpen = ref(false);
const quarantineReason = ref("");
const quarantineGrace = ref(30);
const quarantineLimit = ref(100);
const quarantineGraceInvalid = computed(() => quarantineGrace.value < 10 || quarantineGrace.value > 10080);
const quarantineLimitInvalid = computed(() => quarantineLimit.value < 1 || quarantineLimit.value > 1000);
const quarantineReasonInvalid = computed(() => quarantineReason.value.trim().length === 0);
const quarantineConfirmDisabled = computed(
  () => quarantineReasonInvalid.value || quarantineGraceInvalid.value || quarantineLimitInvalid.value || !!props.action,
);

// Opening the dialog keeps the last result visible but starts a fresh reason;
// closing without confirming leaves stats untouched.
watch(quarantineOpen, (open) => {
  if (!open) return;
  quarantineReason.value = "";
  quarantineGrace.value = 30;
  quarantineLimit.value = 100;
});

function submitQuarantine() {
  if (quarantineConfirmDisabled.value) return;
  emit("quarantine", quarantineReason.value, quarantineGrace.value, quarantineLimit.value);
}

function quarantineResultText(result: QuarantineStaleSubmittingResponse): string {
  return t("taskMaintenance.quarantineResult", {
    quarantined: result.quarantined_count,
    nonTerminal: result.skipped_non_terminal,
    fresh: result.skipped_fresh,
    realRemote: result.skipped_real_remote,
  });
}

function formatMaintenanceCount(value: number | null | undefined): string {
  return value == null ? "--" : String(value);
}

watch(() => props.settings, (settings) => {
  if (!settings) return;
  draftCleanup.value = settings.cleanup_enabled;
  draftRetentionDays.value = settings.retention_days;
}, { immediate: true });

// Reopening the modal discards unsaved edits and restarts from persisted settings.
watch(settingsOpen, (open) => {
  if (!open) return;
  const settings = props.settings;
  if (!settings) return;
  draftCleanup.value = settings.cleanup_enabled;
  draftRetentionDays.value = settings.retention_days;
});

const settingsDirty = computed(() => {
  const settings = props.settings;
  return !!settings && (settings.cleanup_enabled !== draftCleanup.value || settings.retention_days !== draftRetentionDays.value);
});
const retentionInvalid = computed(() => draftRetentionDays.value < 1 || draftRetentionDays.value > 3650);

function saveSettings() {
  if (!settingsDirty.value || retentionInvalid.value || props.saving) return;
  emit("saveSettings", draftCleanup.value, draftRetentionDays.value);
}
</script>

<template>
  <section data-testid="task-maintenance-toolbar" class="flex shrink-0 flex-col gap-2 border-t border-default/70 pt-3">
    <UAlert v-if="error" color="error" variant="subtle" :title="t('common.error')" :description="error" />
    <div class="flex min-w-0 flex-wrap items-center justify-between gap-x-4 gap-y-2">
      <div class="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1">
        <h2 class="text-sm font-semibold text-color">{{ t("taskMaintenance.title") }}</h2>
        <span class="text-sm text-muted">{{ t("taskMaintenance.total") }}: {{ stats?.total ?? "--" }}</span>
        <span class="text-sm text-muted">{{ t("taskMaintenance.active") }}: {{ activeCount }}</span>
        <span class="text-sm text-muted">{{ t("taskMaintenance.expiredTerminal") }}: {{ stats?.expired_terminal ?? "--" }}</span>
        <span class="text-sm text-muted" data-testid="maintenance-uncertain">{{ t("taskMaintenance.uncertainSubmitting") }}: {{ formatMaintenanceCount(uncertainSubmitting) }}</span>
        <span class="text-sm text-muted" data-testid="maintenance-quarantinable">{{ t("taskMaintenance.quarantinableSubmitting") }}: {{ formatMaintenanceCount(quarantinableSubmitting) }}</span>
        <span class="text-sm text-muted" data-testid="maintenance-quarantined">{{ t("taskMaintenance.orphanedJobs") }}: {{ formatMaintenanceCount(orphanedExternalJobs) }}</span>
      </div>
      <div class="flex min-w-0 flex-wrap items-center gap-2">
        <UButton color="error" variant="outline" size="sm" icon="i-lucide-ban" :loading="action === 'cancel'" :disabled="activeCount === 0 || !!action" :label="t('taskMaintenance.cancelActiveAction') + ' (' + activeCount + ')'" @click="emit('confirmCancel')" />
        <UButton color="neutral" variant="outline" size="sm" icon="i-lucide-trash-2" :loading="action === 'purge'" :disabled="!!action" :label="t('taskMaintenance.purgeExpiredAction')" @click="emit('confirmPurge', 'expired')" />
        <UButton color="error" variant="outline" size="sm" icon="i-lucide-trash-2" :loading="action === 'purge'" :disabled="activeCount > 0 || !!action" :label="t('taskMaintenance.purgeAllAction')" @click="emit('confirmPurge', 'all_terminal')" />
        <UButton color="warning" variant="outline" size="sm" icon="i-lucide-shield-alert" :loading="action === 'quarantine'" :disabled="!!action" :label="t('taskMaintenance.quarantine')" data-testid="maintenance-quarantine-button" @click="quarantineOpen = true" />
        <UButton color="neutral" variant="ghost" size="sm" icon="i-lucide-settings" :aria-label="t('taskMaintenance.settings')" :title="t('taskMaintenance.settings')" data-testid="maintenance-settings-button" @click="settingsOpen = true" />
      </div>
    </div>
    <UAlert
      v-if="lastQuarantine"
      color="neutral"
      variant="subtle"
      :title="t('taskMaintenance.quarantineCompleted')"
      :description="lastQuarantine ? quarantineResultText(lastQuarantine) : undefined"
    />
  </section>

  <TaskQuarantineDialog
    :open="quarantineOpen"
    :reason="quarantineReason"
    :grace="quarantineGrace"
    :limit="quarantineLimit"
    :action="action"
    :last-quarantine="lastQuarantine"
    :confirm-disabled="quarantineConfirmDisabled"
    @update:open="quarantineOpen = $event"
    @update:reason="quarantineReason = $event"
    @update:grace="quarantineGrace = $event ?? 30"
    @update:limit="quarantineLimit = $event ?? 100"
    @confirm="submitQuarantine"
  />

  <UModal v-model:open="settingsOpen" :title="t('taskMaintenance.settings')" class="w-[30rem] max-w-[96vw]">
    <template #body>
      <div class="grid gap-3">
        <label class="flex items-center gap-2 text-sm text-color">
          <USwitch :model-value="draftCleanup" :disabled="!settings || saving" data-testid="maintenance-cleanup-toggle" @update:model-value="draftCleanup = $event as boolean" />
          {{ t("taskMaintenance.autoCleanup") }}
        </label>
        <div class="w-36">
          <AppNumberField :input-id="'maintenance-retention'" :label="t('taskMaintenance.retentionDays')" :model-value="draftRetentionDays" :min="1" :max="3650" :disabled="!settings || saving" :test-id="'maintenance-retention'" @update:model-value="draftRetentionDays = $event ?? 30" />
        </div>
      </div>
    </template>
    <template #footer>
      <div class="flex w-full justify-end gap-2">
        <UButton color="neutral" variant="outline" :label="t('common.cancel')" @click="settingsOpen = false" />
        <UButton icon="i-lucide-save" :loading="saving" :disabled="!settingsDirty || retentionInvalid" :label="t('common.save')" @click="saveSettings" />
      </div>
    </template>
  </UModal>
</template>
