<script setup lang="ts">
import { computed } from "vue";
import InputText from "primevue/inputtext";
import IconField from "primevue/iconfield";
import InputIcon from "primevue/inputicon";
import Tag from "primevue/tag";
import Toolbar from "primevue/toolbar";

const props = withDefaults(defineProps<{
  countLabel?: string;
  searchEnabled?: boolean;
  searchPlaceholder?: string;
  searchQuery?: string;
}>(), {
  countLabel: "",
  searchEnabled: false,
  searchPlaceholder: "",
  searchQuery: "",
});

const emit = defineEmits<{
  "update:searchQuery": [value: string];
}>();

const showSearchIcon = computed(() => !(props.searchQuery ?? "").trim());
</script>

<template>
  <Toolbar>
    <template #start>
      <div class="flex min-w-0 flex-wrap items-center gap-2">
        <slot name="main" />
        <Tag v-if="countLabel" :value="countLabel" severity="secondary" />
      </div>
    </template>

    <template #end>
      <div class="flex flex-wrap items-center gap-2">
        <IconField v-if="searchEnabled" class="w-full min-w-0 md:w-72">
          <InputIcon v-if="showSearchIcon" class="pi pi-search" />
          <InputText
            :model-value="searchQuery"
            fluid
            :placeholder="searchPlaceholder"
            @update:model-value="emit('update:searchQuery', String($event ?? ''))"
          />
        </IconField>
        <slot name="actions" />
      </div>
    </template>
  </Toolbar>
</template>
