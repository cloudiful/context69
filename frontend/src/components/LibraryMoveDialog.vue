<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

interface FolderOption {
  value: string | null;
  label: string;
}

const ROOT_OPTION = "__root__";

const props = defineProps<{
  open: boolean;
  busy?: boolean;
  title: string;
  description: string;
  currentFolderId?: string | null;
  options: FolderOption[];
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [targetFolderId: string | null];
}>();

const { t } = useI18n();
const selectedValue = ref(ROOT_OPTION);
const visible = computed({
  get: () => props.open,
  set: (value: boolean) => {
    if (!value) {
      emit("cancel");
    }
  },
});
const selectOptions = computed(() => [
  {
    label: t("library.rootFolder"),
    value: ROOT_OPTION,
  },
  ...props.options.map((option) => ({
    label: option.label,
    value: option.value ?? ROOT_OPTION,
  })),
]);

watch(
  () => [props.open, props.currentFolderId],
  () => {
    selectedValue.value = props.currentFolderId ?? ROOT_OPTION;
  },
  { immediate: true },
);

function confirmMove() {
  emit("confirm", selectedValue.value === ROOT_OPTION ? null : selectedValue.value);
}
</script>

<template>
  <UModal v-model:open="visible" class="w-[34rem] max-w-[92vw]"  :title="title">
    <template #body>
<div class="grid gap-6">
      <div class="grid gap-2">
        <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-color">{{ t("library.moveDialog.label") }}</p>
        <p class="text-sm leading-7 text-muted-color">{{ description }}</p>
      </div>

      <label class="grid gap-2">
        <span class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.moveDialog.targetFolder") }}</span>
        <USelect
          v-model="selectedValue"
          class="w-full"
          :items="selectOptions"
          label-key="label"
          value-key="value"
        />
      </label>
    </div>
    </template>


    <template #footer>
      <div class="flex flex-wrap justify-end gap-3">
        <UButton color="neutral" variant="outline" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </UButton>
        <UButton :disabled="busy" @click="confirmMove">
          {{ busy ? t("library.moving") : t("common.move") }}
        </UButton>
      </div>
    </template>
  </UModal>
</template>
