<script setup lang="ts">
import InputText from "primevue/inputtext";
import IconField from "primevue/iconfield";
import InputIcon from "primevue/inputicon";
import Tag from "primevue/tag";

withDefaults(defineProps<{
  countLabel?: string;
  searchPlaceholder?: string;
  searchQuery?: string;
}>(), {
  countLabel: "",
  searchPlaceholder: "",
  searchQuery: "",
});

const emit = defineEmits<{
  "update:searchQuery": [value: string];
}>();
</script>

<template>
  <div class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-app-border/80 bg-app-surface px-2 py-1.5 max-md:items-stretch max-md:rounded-[0.8rem] max-md:p-2">
    <div class="flex min-w-0 flex-1 flex-wrap items-center gap-2 max-md:w-full">
      <slot name="main" />
      <Tag v-if="countLabel" class="app-table-toolbar-count" :value="countLabel" severity="secondary" />
    </div>

    <div class="flex w-full flex-wrap items-center gap-1.5 md:w-auto md:justify-end">
      <IconField v-if="searchPlaceholder" class="min-w-0 w-full md:w-72">
        <InputIcon class="pi pi-search" />
        <InputText
          :model-value="searchQuery"
          class="w-full min-w-0"
          :placeholder="searchPlaceholder"
          @update:model-value="emit('update:searchQuery', String($event ?? ''))"
        />
      </IconField>

      <slot name="actions" />
    </div>
  </div>
</template>
