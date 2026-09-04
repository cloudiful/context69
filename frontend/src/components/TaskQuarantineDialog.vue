<script setup lang="ts">
import { useI18n } from "vue-i18n";

import type { QuarantineStaleSubmittingResponse } from "../services/api";

defineProps<{
  open: boolean;
  reason: string;
  grace: number;
  limit: number;
  action: "cancel" | "purge" | "quarantine" | null;
  lastQuarantine: QuarantineStaleSubmittingResponse | null;
  confirmDisabled: boolean;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  "update:reason": [value: string];
  "update:grace": [value: number | null];
  "update:limit": [value: number | null];
  confirm: [];
}>();

const { t } = useI18n();

function quarantineResultText(result: QuarantineStaleSubmittingResponse): string {
  return t("taskMaintenance.quarantineResult", {
    quarantined: result.quarantined_count,
    nonTerminal: result.skipped_non_terminal,
    fresh: result.skipped_fresh,
    realRemote: result.skipped_real_remote,
  });
}
</script>

<template>
  <UModal :open="open" :title="t('taskMaintenance.quarantine')" class="w-[30rem] max-w-[96vw]" @update:open="emit('update:open', $event)">
    <template #body>
      <div class="grid gap-3">
        <p class="text-sm text-muted">{{ t("taskMaintenance.quarantineConfirm") }}</p>
        <UInput :model-value="reason" :placeholder="t('taskMaintenance.quarantineReason')" data-testid="maintenance-quarantine-reason" @update:model-value="emit('update:reason', $event as string)" />
        <div class="grid grid-cols-2 gap-3">
          <div class="w-full">
            <AppNumberField :input-id="'maintenance-quarantine-grace'" :label="t('taskMaintenance.quarantineGrace')" :model-value="grace" :min="10" :max="10080" :disabled="action === 'quarantine'" :test-id="'maintenance-quarantine-grace'" @update:model-value="emit('update:grace', $event)" />
          </div>
          <div class="w-full">
            <AppNumberField :input-id="'maintenance-quarantine-limit'" :label="t('taskMaintenance.quarantineLimit')" :model-value="limit" :min="1" :max="1000" :disabled="action === 'quarantine'" :test-id="'maintenance-quarantine-limit'" @update:model-value="emit('update:limit', $event)" />
          </div>
        </div>
        <UAlert
          v-if="lastQuarantine"
          color="neutral"
          variant="subtle"
          :title="t('taskMaintenance.quarantineCompleted')"
          :description="quarantineResultText(lastQuarantine)"
        />
      </div>
    </template>
    <template #footer>
      <div class="flex w-full justify-end gap-2">
        <UButton color="neutral" variant="outline" :label="t('common.cancel')" @click="emit('update:open', false)" />
        <UButton color="warning" icon="i-lucide-shield-alert" :loading="action === 'quarantine'" :disabled="confirmDisabled" :label="t('taskMaintenance.quarantineAction')" data-testid="maintenance-quarantine-confirm" @click="emit('confirm')" />
      </div>
    </template>
  </UModal>
</template>
