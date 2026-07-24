<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

const props = withDefaults(defineProps<{
  page: number;
  pageSize: number;
  pageSizeOptions?: number[];
  total: number;
}>(), {
  pageSizeOptions: () => [10, 25, 50, 100],
});

const emit = defineEmits<{
  "update:page": [number];
  "update:page-size": [number];
}>();

const { t } = useI18n();
const pageSizeItems = computed(() => props.pageSizeOptions.map((value) => ({ label: String(value), value })));

function updatePageSize(value: number) {
  if (value === props.pageSize) return;
  emit("update:page-size", value);
}
</script>

<template>
  <div v-if="props.total > 0" class="flex flex-wrap items-center justify-between gap-2">
    <div class="flex items-center gap-2">
      <span class="text-sm text-muted-color">{{ t("common.itemsPerPage") }}</span>
      <USelect
        :model-value="props.pageSize"
        :items="pageSizeItems"
        value-key="value"
        class="w-20"
        :aria-label="t('common.itemsPerPage')"
        @update:model-value="updatePageSize($event as number)"
      />
    </div>

    <UPagination
      v-if="props.total > props.pageSize"
      :page="props.page"
      :items-per-page="props.pageSize"
      :total="props.total"
      @update:page="emit('update:page', $event)"
    />
  </div>
</template>
