<script setup lang="ts">
import { ref, watch } from "vue";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Message from "primevue/message";

import AppMonacoEditor from "./AppMonacoEditor.vue";

const props = defineProps<{
  busy: boolean;
  error: string;
  folderName?: string;
  folderNameReadonly?: boolean;
  open: boolean;
  title: string;
  value: string;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [{ folderName: string; value: string }];
  "update:value": [string];
}>();

const draftFolderName = ref(props.folderName ?? "");

watch(
  () => [props.folderName, props.open] as const,
  ([folderName, open]) => {
    if (!open) {
      return;
    }
    draftFolderName.value = folderName ?? "";
  },
  { immediate: true },
);
</script>

<template>
  <Dialog
    :visible="open"
    modal
    :header="title"
    class="w-[min(92vw,68rem)]"
    @update:visible="!$event && emit('cancel')"
  >
    <div class="grid gap-4">
      <label class="grid gap-2">
        <span class="text-sm font-medium text-app-text">{{ $t("library.sourceFolderName") }}</span>
        <InputText
          :model-value="draftFolderName"
          fluid
          :disabled="busy"
          :readonly="folderNameReadonly"
          @update:model-value="draftFolderName = String($event)"
        />
      </label>

      <label class="grid gap-2">
        <span class="text-sm font-medium text-app-text">{{ $t("library.sourceConfigLabel") }}</span>
        <div class="min-h-[26rem] overflow-hidden rounded-xl border border-app-border/60">
          <AppMonacoEditor
            :model-value="value"
            language="json"
            @update:model-value="emit('update:value', $event)"
          />
        </div>
      </label>

      <Message v-if="error" severity="error" :closable="false">
        {{ error }}
      </Message>
    </div>

    <template #footer>
      <div class="flex justify-end gap-2">
        <Button severity="secondary" text :disabled="busy" @click="emit('cancel')">
          {{ $t("common.cancel") }}
        </Button>
        <Button :loading="busy" @click="emit('confirm', { folderName: draftFolderName, value })">
          {{ $t("common.save") }}
        </Button>
      </div>
    </template>
  </Dialog>
</template>
