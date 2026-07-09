<script setup lang="ts">
import { computed } from "vue";
import InputText from "primevue/inputtext";
import IconField from "primevue/iconfield";
import InputIcon from "primevue/inputicon";
import Tag from "primevue/tag";

const props = withDefaults(defineProps<{
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

const showSearchIcon = computed(() => !(props.searchQuery ?? "").trim());
</script>

<template>
  <div class="utility-toolbar app-table-toolbar">
    <div class="app-table-toolbar-main">
      <slot name="main" />
      <Tag v-if="countLabel" class="app-table-toolbar-count" :value="countLabel" severity="secondary" />
    </div>

    <div class="utility-toolbar-group app-table-toolbar-actions">
      <IconField v-if="searchPlaceholder" class="relative min-w-0 md:w-72 [&.p-iconfield]:w-full">
        <InputIcon
          v-if="showSearchIcon"
          class="pi pi-search pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-app-text-dim"
        />
        <InputText
          :model-value="searchQuery"
          :class="['w-full min-w-0', showSearchIcon ? 'pl-10' : 'pl-3']"
          :placeholder="searchPlaceholder"
          @update:model-value="emit('update:searchQuery', String($event ?? ''))"
        />
      </IconField>

      <slot name="actions" />
    </div>
  </div>
</template>
