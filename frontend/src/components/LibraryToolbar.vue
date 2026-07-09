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
}>(), {
  countLabel: "",
});

const emit = defineEmits<{
  "update:searchQuery": [value: string];
}>();

const { t } = useI18n();
</script>

<template>
  <AppTableToolbar
    class="library-toolbar-shell"
    :count-label="countLabel"
    :search-placeholder="t('library.filterResourcesPlaceholder')"
    :search-query="searchQuery"
    @update:search-query="emit('update:searchQuery', $event)"
  >
    <template #main>
      <div class="library-toolbar-main">
        <Breadcrumb
          v-if="breadcrumbItems.length > 0"
          :home="breadcrumbHome"
          :model="breadcrumbItems"
          class="library-toolbar-breadcrumb"
        >
          <template #item="{ item }">
            <button
              class="library-breadcrumb-link"
              type="button"
              @click="item.onSelect()"
            >
              {{ item.label }}
            </button>
          </template>
        </Breadcrumb>
      </div>
    </template>
  </AppTableToolbar>
</template>
