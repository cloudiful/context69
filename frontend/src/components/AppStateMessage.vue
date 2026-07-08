<script setup lang="ts">
import Message from "primevue/message";

withDefaults(defineProps<{
  severity?: "success" | "info" | "warn" | "error" | "secondary" | "contrast";
  title?: string;
}>(), {
  severity: "secondary",
  title: "",
});
</script>

<template>
  <Message class="app-state-message" :severity="severity" :closable="false">
    <div class="app-state-message-body">
      <p v-if="title" class="app-state-message-title">
        <strong>{{ title }}</strong>
      </p>
      <p class="app-state-message-text">
        <slot />
      </p>
    </div>
  </Message>
</template>
