<script setup lang="ts">
import Fieldset from "primevue/fieldset";

defineProps<{
  legend: string;
  description?: string;
}>();
</script>

<template>
  <Fieldset
    :legend="legend"
    class="rounded-[1.15rem]"
    :pt="{
      content: { class: 'grid gap-5 p-4 md:p-[1.15rem]' },
      legendLabel: { class: 'text-[0.98rem]' },
    }"
  >
    <div class="grid gap-5">
      <p v-if="description" class="text-sm leading-6 text-app-text-dim">{{ description }}</p>
      <slot />
    </div>
  </Fieldset>
</template>
