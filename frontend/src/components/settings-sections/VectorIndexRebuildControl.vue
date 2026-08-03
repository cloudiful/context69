<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { TaskResponse } from "../../services/api";

const props = defineProps<{
  status: TaskResponse | null;
}>();

defineEmits<{ rebuild: [] }>();

const { t } = useI18n();
const running = computed(() => props.status ? ["queued", "running", "waiting"].includes(props.status.status) : false);
const progress = computed(() => {
  if (!props.status) return "";
  return t(`processingQueue.statuses.${props.status.status}`);
});
</script>

<template>
  <div class="grid gap-2">
    <div>
      <UButton
        size="sm"
        color="neutral"
        :disabled="running"
        :aria-busy="running"
        data-testid="runtime-vector-rebuild"
        @click="$emit('rebuild')"
      >
        <UIcon name="i-lucide-loader-circle" v-if="running" class="h-4 w-4" />
        <UIcon v-else name="i-lucide-refresh-cw" />
        <span>{{ t("settings.runtime.vectorRebuild") }}</span>
      </UButton>
    </div>
    <p v-if="running" class="m-0 text-sm text-muted-color">
      {{ progress || t('settings.runtime.vectorRebuilding') }}
    </p>
    <p v-else-if="status?.status === 'succeeded'" class="m-0 text-sm text-green-500">
      {{ t('settings.runtime.vectorRebuildSucceeded', { count: status.progress.succeeded }) }}
    </p>
    <p v-else-if="status?.status === 'failed'" class="m-0 text-sm text-red-500">
      {{ status.error_summary || t('settings.runtime.vectorRebuildFailed') }}
    </p>
  </div>
</template>
