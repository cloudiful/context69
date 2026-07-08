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
  <Message :severity="severity" :closable="false">
    <div class="grid gap-0.5">
      <p v-if="title" class="m-0">
        <strong>{{ title }}</strong>
      </p>
      <p class="m-0">
        <slot />
      </p>
    </div>
  </Message>
</template>
