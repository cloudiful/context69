<script setup lang="ts">
import Fieldset from "primevue/fieldset";

defineProps<{
  legend: string;
  description?: string;
}>();
</script>

<template>
  <Fieldset class="app-settings-section" :legend="legend">
    <div class="app-settings-section-body">
      <p v-if="description" class="app-settings-section-description">{{ description }}</p>
      <slot />
    </div>
  </Fieldset>
</template>
