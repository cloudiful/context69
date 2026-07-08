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
  <Dialog v-model:visible="visible" class="library-modal" modal :header="t('library.createTextDialog.title')" :style="{ width: '40rem', maxWidth: '92vw' }">
    <div class="library-modal-body">
      <div class="library-modal-intro">
        <p class="section-label">{{ t("library.newTextFile") }}</p>
        <p class="library-modal-description">
          {{ t("library.createTextDialog.description", { name: parentName }) }}
        </p>
      </div>

      <label class="library-modal-field">
        <span class="form-label">{{ t("library.createTextDialog.nameLabel") }}</span>
        <InputText
          id="library-create-text-title"
          v-model="title"
          class="library-modal-control"
          :placeholder="t('library.createTextDialog.namePlaceholder')"
          @keyup.enter="confirmCreate"
        />
      </label>

      <label class="library-modal-field">
        <span class="form-label">{{ t("library.createTextDialog.contentLabel") }}</span>
        <Textarea
          id="library-create-text-content"
          v-model="content"
          class="library-modal-control"
          auto-resize
          rows="10"
          :placeholder="t('library.createTextDialog.contentPlaceholder')"
        />
      </label>
    </div>

    <template #footer>
      <div class="library-modal-footer">
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
