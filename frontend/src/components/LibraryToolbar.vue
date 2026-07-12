<script setup lang="ts">
import { computed } from "vue";
import Breadcrumb from "primevue/breadcrumb";
import type { MenuItem } from "primevue/menuitem";

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

const breadcrumbHomeItem = computed<MenuItem>(() => ({
  label: props.breadcrumbHome.label,
  command: props.breadcrumbHome.onSelect,
}));
const breadcrumbModel = computed<MenuItem[]>(() => props.breadcrumbItems.map((item) => ({
  label: item.label,
  command: item.onSelect,
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
        <Breadcrumb
          v-if="breadcrumbItems.length > 0"
          :home="breadcrumbHomeItem"
          :model="breadcrumbModel"
          class="min-w-0"
        />
      </div>
    </template>
    <template #actions>
      <slot name="actions" />
    </template>
  </AppTableToolbar>
</template>
