<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

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
  <UModal
    :open="visible"

    :title="title"
    class="w-[32rem] max-w-[96vw]"
    @update:open="emit('update:visible', $event)"
  >
    <template #body>
<div class="grid gap-3">
      <div v-if="showKey" class="grid gap-2">
        <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ entityKeyLabel || t("groups.groupKey") }}</label>
        <UInput
          v-model="entityKey"
          :placeholder="entityKeyLabel || t('groups.groupKey')"
        />
      </div>

      <div class="grid gap-2">
        <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ entityNameLabel || t("groups.groupName") }}</label>
        <UInput
          v-model="entityName"
          :placeholder="entityNameLabel || t('groups.groupName')"
        />
      </div>

      <div class="grid gap-2">
        <label class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("groups.visibility") }}</label>
        <USelect
          v-model="visibility"
          label-key="label"
          value-key="value"
          :items="visibilityOptions"
        />
      </div>
    </div>
    </template>


    <template #footer>
      <div class="flex justify-end gap-2">
        <UButton color="neutral" variant="outline" :disabled="busy" @click="close">
          {{ t("common.cancel") }}
        </UButton>
        <UButton :disabled="busy || !canSubmit" @click="handleSubmit">
          {{ submitLabel || t("common.save") }}
        </UButton>
      </div>
    </template>
  </UModal>
</template>
