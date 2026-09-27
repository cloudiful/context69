<script setup lang="ts">
import { ref, watch } from "vue";

export interface ProjectFilesFileUploadHandle {
  trigger: () => void;
}

type FileUploadController = {
  inputRef?: HTMLInputElement;
};

const emit = defineEmits<{
  "select": [payload: { files: File[] }];
}>();

const fileUpload = ref<FileUploadController | null>(null);
const uploadFiles = ref<File[] | null>(null);

function trigger() {
  fileUpload.value?.inputRef?.click();
}

watch(uploadFiles, (files) => {
  if (!files?.length) return;
  emit("select", { files });
  uploadFiles.value = null;
});

defineExpose<ProjectFilesFileUploadHandle>({ trigger });
</script>

<template>
  <UFileUpload
    ref="fileUpload"
    v-model="uploadFiles"
    class="sr-only"
    multiple
    :preview="false"
    :dropzone="false"
    accept=".pdf,.docx,.xlsx,.md,.txt,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,text/plain,text/markdown"
  />
</template>
