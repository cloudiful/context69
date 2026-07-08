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
  <div class="utility-toolbar app-table-toolbar">
    <div class="app-table-toolbar-main">
      <slot name="main" />
      <Tag v-if="countLabel" class="app-table-toolbar-count" :value="countLabel" severity="secondary" />
    </div>

    <div class="utility-toolbar-group app-table-toolbar-actions">
      <IconField v-if="searchPlaceholder" class="app-table-toolbar-search">
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
