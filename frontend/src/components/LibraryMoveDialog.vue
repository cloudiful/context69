<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import Select from "primevue/select";

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
  <Dialog v-model:visible="visible" class="library-modal w-[34rem] max-w-[92vw]" modal :header="title">
    <div class="grid gap-6">
      <div class="grid gap-2">
        <p class="text-xs font-medium uppercase tracking-[0.18em] text-app-text-dim">{{ t("library.moveDialog.label") }}</p>
        <p class="text-sm leading-7 text-app-text-muted">{{ description }}</p>
      </div>

      <label class="grid gap-2">
        <span class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ t("library.moveDialog.targetFolder") }}</span>
        <Select
          v-model="selectedValue"
          class="w-full"
          :options="selectOptions"
          option-label="label"
          option-value="value"
        />
      </label>
    </div>

    <template #footer>
      <div class="flex flex-wrap justify-end gap-3">
        <Button severity="secondary" variant="outlined" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </Button>
        <Button :disabled="busy" @click="confirmMove">
          {{ busy ? t("library.moving") : t("common.move") }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
