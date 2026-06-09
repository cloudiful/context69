<script setup lang="ts">
defineProps<{
  inputId: string;
  label: string;
  helper?: string;
}>();
</script>

<template>
  <div class="app-form-field">
    <label class="app-form-field-label" :for="inputId">{{ label }}</label>
    <slot />
    <p v-if="helper" class="app-form-field-help">{{ helper }}</p>
  </div>
</template>
