<script setup lang="ts">
const props = withDefaults(defineProps<{
  countLabel?: string;
  searchEnabled?: boolean;
  searchPlaceholder?: string;
  searchQuery?: string;
}>(), {
  countLabel: "",
  searchEnabled: false,
  searchPlaceholder: "",
  searchQuery: "",
});

const emit = defineEmits<{
  "update:searchQuery": [value: string];
}>();

</script>

<template>
  <UDashboardToolbar class="flex-wrap justify-between gap-2">
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <slot name="main" />
      <UBadge v-if="countLabel" :label="countLabel" color="neutral" variant="subtle" />
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <UInput
        v-if="searchEnabled"
        class="w-full min-w-0 md:w-72"
        icon="i-lucide-search"
        :model-value="searchQuery"
        :placeholder="searchPlaceholder"
        @update:model-value="emit('update:searchQuery', String($event ?? ''))"
      />
      <slot name="actions" />
    </div>
  </UDashboardToolbar>
</template>
