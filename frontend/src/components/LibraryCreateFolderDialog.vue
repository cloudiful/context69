<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

const props = defineProps<{
  open: boolean;
  busy?: boolean;
  parentName: string;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [name: string];
}>();

const { t } = useI18n();
const folderName = ref("");
const visible = computed({
  get: () => props.open,
  set: (value: boolean) => {
    if (!value) {
      emit("cancel");
    }
  },
});
const trimmedName = computed(() => folderName.value.trim());

watch(
  () => props.open,
  (open) => {
    if (open) {
      folderName.value = "";
    }
  },
);

function confirmCreate() {
  if (!trimmedName.value) {
    return;
  }

  emit("confirm", trimmedName.value);
}
</script>

<template>
  <UModal v-model:open="visible" class="w-[32rem] max-w-[92vw]"  :title="t('library.createDialog.title')">
    <template #body>
<div class="grid gap-6">
      <div class="grid gap-2">
        <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-color">{{ t("library.newFolder") }}</p>
        <p class="text-sm leading-7 text-muted-color">
          {{ t("library.createDialog.description", { name: parentName }) }}
        </p>
      </div>

      <label class="grid gap-2">
        <span class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-muted-color">{{ t("library.createDialog.nameLabel") }}</span>
        <UInput
          id="library-create-folder-name"
          v-model="folderName"
          class="w-full"
          :placeholder="t('library.newFolderPlaceholder')"
          @keyup.enter="confirmCreate"
        />
      </label>
    </div>
    </template>


    <template #footer>
      <div class="flex flex-wrap justify-end gap-3">
        <UButton color="neutral" variant="outline" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </UButton>
        <UButton :disabled="busy || !trimmedName" @click="confirmCreate">
          {{ busy ? t("library.creating") : t("library.createDialog.submit") }}
        </UButton>
      </div>
    </template>
  </UModal>
</template>
