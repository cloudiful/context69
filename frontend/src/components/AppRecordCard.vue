<script setup lang="ts">
import Card from "primevue/card";

import AppRecordHeader from "./AppRecordHeader.vue";

defineProps<{
  description?: string;
  subtitle?: string;
  title: string;
}>();
</script>

<template>
  <Card>
    <template #title>
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

    <template #content>
      <slot />
    </template>
  </Card>
</template>
