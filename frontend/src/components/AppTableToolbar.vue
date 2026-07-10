<script setup lang="ts">
import { computed } from "vue";
import InputText from "primevue/inputtext";
import IconField from "primevue/iconfield";
import InputIcon from "primevue/inputicon";
import Tag from "primevue/tag";

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
  <div class="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-app-border/80 bg-app-surface px-2 py-1.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)]">
    <div class="flex min-w-0 flex-1 flex-wrap items-center gap-2">
      <slot name="main" />
      <Tag v-if="countLabel" class="rounded-md px-2 py-1 text-xs font-semibold" :value="countLabel" severity="secondary" />
    </div>

    <div class="flex w-full flex-wrap items-center gap-1.5 md:w-auto md:justify-end">
      <IconField v-if="searchEnabled" class="relative min-w-0 md:w-72 [&.p-iconfield]:w-full">
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
