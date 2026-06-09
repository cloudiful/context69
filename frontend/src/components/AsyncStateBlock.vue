<script setup lang="ts">
import ProgressSpinner from "primevue/progressspinner";

import AppStateMessage from "./AppStateMessage.vue";
import EmptyState from "./EmptyState.vue";

withDefaults(defineProps<{
  empty?: boolean;
  emptyMessage?: string;
  emptyTitle?: string;
  emptyVariant?: "default" | "soft";
  error?: string | null;
  errorSeverity?: "success" | "info" | "warn" | "error" | "secondary" | "contrast";
  errorTitle?: string;
  loading?: boolean;
  loadingMessage?: string;
  loadingTestId?: string;
  loadingTitle?: string;
}>(), {
  empty: false,
  emptyMessage: "",
  emptyTitle: "",
  emptyVariant: "default",
  error: "",
  errorSeverity: "error",
  errorTitle: "",
  loading: false,
  loadingMessage: "",
  loadingTestId: "",
  loadingTitle: "",
});
</script>

<template>
  <slot v-if="loading" name="loading">
    <div class="flex flex-col items-center justify-center gap-4 py-12 text-center">
      <ProgressSpinner :data-testid="loadingTestId || undefined" stroke-width="4" style="width: 3rem; height: 3rem" />
      <div>
        <p v-if="loadingTitle">{{ loadingTitle }}</p>
        <p v-if="loadingMessage">{{ loadingMessage }}</p>
      </div>
    </div>
  </slot>

  <slot v-else-if="error" name="error" :error="error">
    <AppStateMessage :severity="errorSeverity" :title="errorTitle">
      {{ error }}
    </AppStateMessage>
  </slot>

  <slot v-else-if="empty" name="empty">
    <EmptyState
      :title="emptyTitle"
      :message="emptyMessage"
      :variant="emptyVariant"
    />
  </slot>

  <slot v-else />
</template>
