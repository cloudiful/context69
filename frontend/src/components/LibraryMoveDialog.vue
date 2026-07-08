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
  <Dialog v-model:visible="visible" modal :header="title" :style="{ width: '34rem', maxWidth: '92vw' }">
    <div class="grid gap-4">
      <div class="grid gap-1.5">
        <p class="section-label text-xs font-semibold uppercase tracking-[0.16em] text-app-text-dim">{{ t("library.moveDialog.label") }}</p>
        <p class="text-sm leading-6 text-app-text-muted">{{ description }}</p>
      </div>

      <label class="grid gap-1.5">
        <span class="text-sm font-medium text-app-text">{{ t("library.moveDialog.targetFolder") }}</span>
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
      <div class="flex justify-end gap-2">
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
