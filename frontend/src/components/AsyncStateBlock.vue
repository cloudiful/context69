<script setup lang="ts">
import EmptyState from "./EmptyState.vue";

withDefaults(defineProps<{
  empty?: boolean;
  emptyMessage?: string;
  emptyTitle?: string;
  emptyVariant?: "default" | "soft";
  error?: string | null;
  errorColor?: "success" | "info" | "warning" | "error" | "neutral";
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
  errorColor: "error",
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
      <UIcon name="i-lucide-loader-circle" :data-testid="loadingTestId || undefined" class="h-12 w-12 animate-spin" />
      <div>
        <p v-if="loadingTitle">{{ loadingTitle }}</p>
        <p v-if="loadingMessage">{{ loadingMessage }}</p>
      </div>
    </div>
  </slot>

  <slot v-else-if="error" name="error" :error="error">
    <UAlert :color="errorColor" variant="subtle" :title="errorTitle || undefined" :description="error" />
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
