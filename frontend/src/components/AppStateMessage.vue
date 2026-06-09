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
    <div>
      <p v-if="title">
        <strong>{{ title }}</strong>
      </p>
      <p>
        <slot />
      </p>
    </div>
  </Message>
</template>
