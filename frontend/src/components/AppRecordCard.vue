<script setup lang="ts">
import AppRecordHeader from "./AppRecordHeader.vue";

defineProps<{
  description?: string;
  subtitle?: string;
  title: string;
}>();
</script>

<template>
  <UCard>
    <template #header>
      <AppRecordHeader
        :title="title"
        :subtitle="subtitle"
        :description="description"
      >
        <template v-if="$slots.tags" #tags>
          <slot name="tags" />
        </template>

        <template v-if="$slots.meta" #meta>
          <slot name="meta" />
        </template>

        <template v-if="$slots.actions" #actions>
          <slot name="actions" />
        </template>
      </AppRecordHeader>
    </template>

    <slot />
  </UCard>
</template>
