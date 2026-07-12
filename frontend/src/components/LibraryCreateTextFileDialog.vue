<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Textarea from "primevue/textarea";

const props = defineProps<{
  open: boolean;
  busy?: boolean;
  parentName: string;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [payload: { title: string; content: string }];
}>();

const { t } = useI18n();
const title = ref("");
const content = ref("");
const visible = computed({
  get: () => props.open,
  set: (value: boolean) => {
    if (!value) {
      emit("cancel");
    }
  },
});
const trimmedTitle = computed(() => title.value.trim());

watch(
  () => props.open,
  (open) => {
    if (open) {
      title.value = "";
      content.value = "";
    }
  },
);

function confirmCreate() {
  if (!trimmedTitle.value) {
    return;
  }

  emit("confirm", {
    title: trimmedTitle.value,
    content: content.value,
  });
}
</script>

<template>
  <Dialog v-model:visible="visible" class="w-[40rem] max-w-[92vw]" modal :header="t('library.createTextDialog.title')">
    <div class="grid gap-6">
      <div class="grid gap-2">
        <p class="text-xs font-medium uppercase tracking-[0.18em] text-(--p-text-muted-color)">{{ t("library.newTextFile") }}</p>
        <p class="text-sm leading-7 text-(--p-text-muted-color)">
          {{ t("library.createTextDialog.description", { name: parentName }) }}
        </p>
      </div>

      <label class="grid gap-2">
        <span class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.createTextDialog.nameLabel") }}</span>
        <InputText
          id="library-create-text-title"
          v-model="title"
          class="w-full"
          :placeholder="t('library.createTextDialog.namePlaceholder')"
          @keyup.enter="confirmCreate"
        />
      </label>

      <label class="grid gap-2">
        <span class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-(--p-text-muted-color)">{{ t("library.createTextDialog.contentLabel") }}</span>
        <Textarea
          id="library-create-text-content"
          v-model="content"
          class="w-full"
          auto-resize
          rows="10"
          :placeholder="t('library.createTextDialog.contentPlaceholder')"
        />
      </label>
    </div>

    <template #footer>
      <div class="flex flex-wrap justify-end gap-3">
        <Button severity="secondary" variant="outlined" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </Button>
        <Button :disabled="busy || !trimmedTitle" @click="confirmCreate">
          {{ busy ? t("library.creating") : t("library.createTextDialog.submit") }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
