<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";

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
  <Dialog v-model:visible="visible" class="library-modal" modal :header="t('library.createDialog.title')" :style="{ width: '32rem', maxWidth: '92vw' }">
    <div class="library-modal-body">
      <div class="library-modal-intro">
        <p class="section-label">{{ t("library.newFolder") }}</p>
        <p class="library-modal-description">
          {{ t("library.createDialog.description", { name: parentName }) }}
        </p>
      </div>

      <label class="library-modal-field">
        <span class="form-label">{{ t("library.createDialog.nameLabel") }}</span>
        <InputText
          id="library-create-folder-name"
          v-model="folderName"
          class="library-modal-control"
          :placeholder="t('library.newFolderPlaceholder')"
          @keyup.enter="confirmCreate"
        />
      </label>
    </div>

    <template #footer>
      <div class="library-modal-footer">
        <Button severity="secondary" variant="outlined" :disabled="busy" @click="emit('cancel')">
          {{ t("common.cancel") }}
        </Button>
        <Button :disabled="busy || !trimmedName" @click="confirmCreate">
          {{ busy ? t("library.creating") : t("library.createDialog.submit") }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
