<script setup lang="ts">
import LibraryCreateFolderDialog from "./LibraryCreateFolderDialog.vue";
import LibraryCreateTextFileDialog from "./LibraryCreateTextFileDialog.vue";
import LibraryMoveDialog from "./LibraryMoveDialog.vue";
import ProjectSourceFolderDialog from "./ProjectSourceFolderDialog.vue";
import type { ProjectFilesLibraryDialogsBindings } from "../composables/project-library/use-project-files-dialogs";

const props = defineProps<ProjectFilesLibraryDialogsBindings>();

const emit = defineEmits<{
  "create-folder-cancel": [];
  "create-folder-confirm": [name: string];
  "create-text-file-cancel": [];
  "create-text-file-confirm": [payload: { title: string; content: string }];
  "move-cancel": [];
  "move-confirm": [targetFolderId: string | null];
  "source-folder-cancel": [];
  "source-folder-confirm": [payload: { folderName: string; value: string }];
  "source-folder-update-value": [value: string];
}>();
</script>

<template>
  <LibraryCreateFolderDialog
    v-bind="props.createFolder"
    @cancel="emit('create-folder-cancel')"
    @confirm="emit('create-folder-confirm', $event)"
  />

  <LibraryCreateTextFileDialog
    v-bind="props.createTextFile"
    @cancel="emit('create-text-file-cancel')"
    @confirm="emit('create-text-file-confirm', $event)"
  />

  <LibraryMoveDialog
    v-bind="props.move"
    @cancel="emit('move-cancel')"
    @confirm="emit('move-confirm', $event)"
  />

  <ProjectSourceFolderDialog
    v-bind="props.sourceFolder"
    @cancel="emit('source-folder-cancel')"
    @confirm="emit('source-folder-confirm', $event)"
    @update:value="emit('source-folder-update-value', $event)"
  />
</template>
