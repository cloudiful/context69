<script setup lang="ts">
import { computed } from "vue";
import type { BreadcrumbItem as NuxtBreadcrumbItem } from "@nuxt/ui";


interface BreadcrumbItem {
  label: string;
  onSelect: () => void;
}

const props = withDefaults(defineProps<{
  breadcrumbHome: BreadcrumbItem;
  breadcrumbItems: BreadcrumbItem[];
  countLabel?: string;
  searchQuery: string;
  showSearch?: boolean;
}>(), {
  countLabel: "",
  showSearch: true,
});

const emit = defineEmits<{
  "update:searchQuery": [value: string];
}>();

const breadcrumbModel = computed<NuxtBreadcrumbItem[]>(() => [props.breadcrumbHome, ...props.breadcrumbItems].map((item) => ({
  label: item.label,
  onSelect: item.onSelect,
})));
</script>

<template>
  <UDashboardToolbar
    class="library-toolbar-shell"
  >
    <div class="flex min-w-0 flex-1 items-center gap-3 overflow-hidden">
      <UBreadcrumb
        v-if="breadcrumbItems.length > 0"
        :items="breadcrumbModel"
        class="min-w-0"
      />
      <UBadge v-if="countLabel" :label="countLabel" color="neutral" variant="subtle" />
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <UInput
        v-if="showSearch"
        class="w-full min-w-0 md:w-72"
        icon="i-lucide-search"
        :model-value="searchQuery"
        @update:model-value="emit('update:searchQuery', String($event ?? ''))"
      />
      <slot name="actions" />
    </div>
  </UDashboardToolbar>
</template>
