<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Select from "primevue/select";

import { appFormDialogPt } from "./app-dialog";
import type { Visibility } from "../services/api";

const props = defineProps<{
  visible: boolean;
  title: string;
  busy?: boolean;
  submitLabel?: string;
  showKey?: boolean;
  entityKeyLabel?: string;
  entityNameLabel?: string;
  initialKey?: string;
  initialName?: string;
  initialVisibility?: Visibility;
}>();

const emit = defineEmits<{
  "update:visible": [boolean];
  submit: [{ key?: string; name: string; visibility: Visibility }];
}>();

const { t } = useI18n();
const entityKey = ref("");
const entityName = ref("");
const visibility = ref<Visibility>("private");

const visibilityOptions = computed(() => [
  { label: t("groups.visibilityOptions.private"), value: "private" },
  { label: t("groups.visibilityOptions.public"), value: "public" },
]);

watch(
  () => props.visible,
  (visible) => {
    if (!visible) return;
    entityKey.value = props.initialKey ?? "";
    entityName.value = props.initialName ?? "";
    visibility.value = props.initialVisibility ?? "private";
  },
  { immediate: true },
);

const canSubmit = computed(() => {
  if (props.showKey && !entityKey.value.trim()) return false;
  return entityName.value.trim().length > 0;
});

function close() {
  emit("update:visible", false);
}

function handleSubmit() {
  emit("submit", {
    key: props.showKey ? entityKey.value.trim() : undefined,
    name: entityName.value.trim(),
    visibility: visibility.value,
  });
}
</script>

<template>
  <Dialog
    :visible="visible"
    modal
    :header="title"
    :pt="appFormDialogPt"
    class="w-[32rem] max-w-[96vw]"
    @update:visible="emit('update:visible', $event)"
  >
    <div class="grid gap-3">
      <div v-if="showKey" class="grid gap-2">
        <label class="form-label">{{ entityKeyLabel || t("groups.groupKey") }}</label>
        <InputText
          v-model="entityKey"
          :placeholder="entityKeyLabel || t('groups.groupKey')"
        />
      </div>

      <div class="grid gap-2">
        <label class="form-label">{{ entityNameLabel || t("groups.groupName") }}</label>
        <InputText
          v-model="entityName"
          :placeholder="entityNameLabel || t('groups.groupName')"
        />
      </div>

      <div class="grid gap-2">
        <label class="form-label">{{ t("groups.visibility") }}</label>
        <Select
          v-model="visibility"
          fluid
          option-label="label"
          option-value="value"
          :options="visibilityOptions"
        />
      </div>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <Button severity="secondary" variant="outlined" :disabled="busy" @click="close">
          {{ t("common.cancel") }}
        </Button>
        <Button :disabled="busy || !canSubmit" @click="handleSubmit">
          {{ submitLabel || t("common.save") }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
