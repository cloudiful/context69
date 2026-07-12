<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import ProgressSpinner from "primevue/progressspinner";

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
      <Button
        size="small"
        severity="secondary"
        :disabled="running"
        :aria-busy="running"
        data-testid="runtime-vector-rebuild"
        @click="$emit('rebuild')"
      >
        <ProgressSpinner v-if="running" class="h-4 w-4" :stroke-width="6" />
        <i v-else class="pi pi-refresh" aria-hidden="true" />
        <span>{{ t("settings.runtime.vectorRebuild") }}</span>
      </Button>
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
