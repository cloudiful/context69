<script setup lang="ts">
import { ref, watch } from "vue";

import AppMonacoEditor from "./AppMonacoEditor.vue";

const props = defineProps<{
  busy: boolean;
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
  <UModal
    :open="open"

    :title="title"
    class="w-[min(92vw,68rem)]"
    @update:open="!$event && emit('cancel')"
  >
    <template #body>
<div class="grid gap-4">
      <label class="grid gap-2">
        <span class="text-sm font-medium text-color">{{ $t("library.sourceFolderName") }}</span>
        <UInput
          :model-value="draftFolderName"
          :disabled="busy"
          :readonly="folderNameReadonly"
          @update:model-value="draftFolderName = String($event)"
        />
      </label>

      <label class="grid gap-2">
        <span class="text-sm font-medium text-color">{{ $t("library.sourceConfigLabel") }}</span>
        <div class="min-h-[26rem] overflow-hidden rounded-xl border border-surface">
          <AppMonacoEditor
            :model-value="value"
            language="json"
            @update:model-value="emit('update:value', $event)"
          />
        </div>
      </label>

    </div>
    </template>


    <template #footer>
      <div class="flex justify-end gap-2">
        <UButton color="neutral" variant="ghost" :disabled="busy" @click="emit('cancel')">
          {{ $t("common.cancel") }}
        </UButton>
        <UButton :disabled="busy" :aria-busy="busy" @click="emit('confirm', { folderName: draftFolderName, value })">
          <UIcon name="i-lucide-loader-circle" v-if="busy" class="h-4 w-4" />
          {{ $t("common.save") }}
        </UButton>
      </div>
    </template>
  </UModal>
</template>
