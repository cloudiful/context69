<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { BreadcrumbItem as NuxtBreadcrumbItem } from "@nuxt/ui";


interface BreadcrumbItem {
  label: string;
  onSelect: () => void;
}

const props = withDefaults(defineProps<{
  breadcrumbHome: BreadcrumbItem;
  breadcrumbItems: BreadcrumbItem[];
  canGoUp?: boolean;
  countLabel?: string;
  searchQuery: string;
  showSearch?: boolean;
}>(), {
  canGoUp: false,
  countLabel: "",
  showSearch: true,
});

const emit = defineEmits<{
  "update:searchQuery": [value: string];
  up: [];
}>();

const { t } = useI18n();

const breadcrumbModel = computed<NuxtBreadcrumbItem[]>(() => [props.breadcrumbHome, ...props.breadcrumbItems].map((item) => ({
  label: item.label,
  onSelect: item.onSelect,
})));
</script>

<template>
  <UDashboardToolbar>
    <div class="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
      <UButton
        data-testid="library-go-up"
        color="neutral"
        variant="ghost"
        size="sm"
        icon="i-lucide-arrow-up"
        :disabled="!canGoUp"
        :aria-label="t('library.goUp')"
        :title="t('library.goUp')"
        class="shrink-0"
        @click="emit('up')"
      />
      <UBreadcrumb
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
