<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Breadcrumb from "primevue/breadcrumb";

import AppTableToolbar from "./AppTableToolbar.vue";

interface BreadcrumbItem {
  label: string;
  onSelect: () => void;
}

withDefaults(defineProps<{
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

const { t } = useI18n();
</script>

<template>
  <AppTableToolbar
    class="library-toolbar-shell px-0 py-1 shadow-none"
    :count-label="countLabel"
    :search-enabled="showSearch"
    :search-query="searchQuery"
    @update:search-query="emit('update:searchQuery', $event)"
  >
    <template #main>
      <div class="flex min-w-0 flex-1 items-center gap-3 overflow-hidden">
        <Breadcrumb
          v-if="breadcrumbItems.length > 0"
          :home="breadcrumbHome"
          :model="breadcrumbItems"
          class="min-w-0"
        >
          <template #item="{ item }">
            <button
              class="text-left text-sm font-medium text-app-text transition hover:text-app-text-muted"
              type="button"
              @click="item.onSelect()"
            >
              {{ item.label }}
            </button>
          </template>
        </Breadcrumb>
      </div>
    </template>
    <template #actions>
      <slot name="actions" />
    </template>
  </AppTableToolbar>
</template>
