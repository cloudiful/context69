<script setup lang="ts">
import { computed } from "vue";
import type { BreadcrumbItem as NuxtBreadcrumbItem } from "@nuxt/ui";

import AppTableToolbar from "./AppTableToolbar.vue";

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
  <AppTableToolbar
    class="library-toolbar-shell"
    :count-label="countLabel"
    :search-enabled="showSearch"
    :search-query="searchQuery"
    @update:search-query="emit('update:searchQuery', $event)"
  >
    <template #main>
      <div class="flex min-w-0 flex-1 items-center gap-3 overflow-hidden">
        <UBreadcrumb
          v-if="breadcrumbItems.length > 0"
          :items="breadcrumbModel"
          class="min-w-0"
        />
      </div>
    </template>
    <template #actions>
      <slot name="actions" />
    </template>
  </AppTableToolbar>
</template>
