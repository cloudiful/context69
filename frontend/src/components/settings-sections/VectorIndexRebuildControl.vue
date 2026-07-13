<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { VectorIndexRebuildStatus } from "../../services/api";

const props = defineProps<{
  status: VectorIndexRebuildStatus | null;
}>();

defineEmits<{ rebuild: [] }>();

const { t } = useI18n();
const running = computed(() => props.status?.state === "running");
const progress = computed(() => {
  if (!props.status || props.status.total_chunks === 0) return "";
  return t("settings.runtime.vectorRebuildProgress", {
    processed: props.status.processed_chunks,
    total: props.status.total_chunks,
  });
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
    <p v-else-if="status?.state === 'succeeded'" class="m-0 text-sm text-green-500">
      {{ t('settings.runtime.vectorRebuildSucceeded', { count: status.processed_chunks }) }}
    </p>
    <p v-else-if="status?.state === 'failed'" class="m-0 text-sm text-red-500">
      {{ status.error_message || t('settings.runtime.vectorRebuildFailed') }}
    </p>
  </div>
</template>
