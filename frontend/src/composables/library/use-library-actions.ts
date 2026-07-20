import { computed, ref, type Ref } from "vue";
import { useToast } from "@nuxt/ui/composables";
import { useAppConfirm } from "../use-app-confirm";

import { apiClient, type LibraryFileSummary, type LibraryFolderNode } from "../../services/api";
import type { ExplorerEntry } from "../../types/library";
import { collectDescendantFolderIds } from "../../utils/library-tree";
import { useErrorToast } from "../use-error-toast";

interface MoveDialogState {
  kind: "file" | "folder";
  id: string;
  name: string;
  currentFolderId: string | null;
  excludedFolderIds: string[];
}

interface CreateDialogState {
  parentFolderId: string | null;
  parentFolderName: string;
}

interface LibraryMenuItem {
  command: () => void;
  danger?: boolean;
  icon: string;
  label: string;
}

interface UseLibraryActionsOptions {
  loadTree: () => Promise<void>;
  moveOptions: Ref<Array<{ label: string; value: string | null }>>;
  replaceQuery: (folderId: string | null, fileId: string | null) => Promise<void>;
  selectFile: (fileId: string) => Promise<void>;
  selectedFolder: Ref<LibraryFolderNode | null>;
  selectedFileId: Ref<string | null>;
  t: (key: string, params?: Record<string, unknown>) => string;
  updateExpandedForFolder: (folderId: string | null) => void;
  previewDocked: Ref<boolean>;
  previewDialogVisible: Ref<boolean>;
}

export function useLibraryActions({
  loadTree,
  moveOptions,
  replaceQuery,
  selectFile,
  selectedFolder,
  selectedFileId,
  t,
  updateExpandedForFolder,
  previewDocked,
  previewDialogVisible,
}: UseLibraryActionsOptions) {
  const confirm = useAppConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();

  const createFolderBusy = ref(false);
  const uploadBusy = ref(false);
  const actionBusy = ref(false);
  const moveDialog = ref<MoveDialogState | null>(null);
  const createDialog = ref<CreateDialogState | null>(null);

  function openCreateFolderDialog(folder: LibraryFolderNode | null = selectedFolder.value) {
    if (!folder) {
      return;
    }

    createDialog.value = {
      parentFolderId: folder.folder_id ?? null,
      parentFolderName: folder.name,
    };
  }

  async function confirmCreateFolder(name: string) {
    if (!createDialog.value) {
      return;
    }

    const parentFolderId = createDialog.value.parentFolderId;
    createFolderBusy.value = true;

    try {
      const folder = await apiClient.createLibraryFolder({
        parent_folder_id: parentFolderId,
        name,
      });
      createDialog.value = null;
      await loadTree();
      updateExpandedForFolder(parentFolderId);
      updateExpandedForFolder(folder.folder_id ?? null);
      await replaceQuery(folder.folder_id, null);
      toast.add({
        color: "success",
        title: t("library.newFolder"),
        description: folder.name,
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("library.createFolderFailed"));
    } finally {
      createFolderBusy.value = false;
    }
  }

  async function handleFileSelection(event: { files?: File[] }) {
    const files = Array.from(event.files ?? []);
    if (files.length === 0) {
      return;
    }

    uploadBusy.value = true;

    try {
      const response = await apiClient.uploadLibraryFiles(selectedFolder.value?.folder_id ?? null, files);
      await loadTree();

      if (response.files.length > 0) {
        await replaceQuery(response.files[0].folder_id ?? selectedFolder.value?.folder_id ?? null, response.files[0].file_id);
      }

      toast.add({
        color: "success",
        title: t("common.upload"),
        description: t("library.uploadSuccess"),
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("library.uploadFailed"));
    } finally {
      uploadBusy.value = false;
    }
  }

  async function revealPreviewForFile(fileId: string) {
    previewDialogVisible.value = !previewDocked.value;
    await selectFile(fileId);
  }

  function openMoveFolderDialog(folder: LibraryFolderNode) {
    if (!folder.folder_id) {
      return;
    }

    moveDialog.value = {
      kind: "folder",
      id: folder.folder_id,
      name: folder.name,
      currentFolderId: folder.parent_folder_id ?? null,
      excludedFolderIds: collectDescendantFolderIds(folder),
    };
  }

  function openMoveFileDialog(file: LibraryFileSummary) {
    moveDialog.value = {
      kind: "file",
      id: file.file_id,
      name: file.filename,
      currentFolderId: selectedFolder.value?.folder_id ?? null,
      excludedFolderIds: [],
    };
  }

  async function confirmMove(targetFolderId: string | null) {
    if (!moveDialog.value) {
      return;
    }

    actionBusy.value = true;

    try {
      if (moveDialog.value.kind === "folder") {
        await apiClient.moveLibraryFolder(moveDialog.value.id, {
          target_folder_id: targetFolderId,
        });
        await loadTree();
        await replaceQuery(targetFolderId, null);
      } else {
        await apiClient.moveLibraryFile(moveDialog.value.id, {
          target_folder_id: targetFolderId,
        });
        await loadTree();
        await replaceQuery(targetFolderId, moveDialog.value.id);
      }
      toast.add({
        color: "success",
        title: t("common.move"),
        description: moveDialog.value.name,
        duration: 2500,
      });
      moveDialog.value = null;
    } catch (error) {
      showErrorToast(error, t("library.moveFailed"));
    } finally {
      actionBusy.value = false;
    }
  }

  function deleteFolder(folder: LibraryFolderNode) {
    if (!folder.folder_id) {
      return;
    }

    confirm.require({
      header: t("common.delete"),
      message: t("library.deleteFolderConfirm", { name: folder.name }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("common.delete"),
      accept: () => {
        void deleteFolderConfirmed(folder);
      },
    });
  }

  async function deleteFolderConfirmed(folder: LibraryFolderNode) {
    actionBusy.value = true;

    try {
      await apiClient.deleteLibraryFolder(folder.folder_id!);
      await loadTree();
      await replaceQuery(null, null);
      toast.add({
        color: "success",
        title: t("common.delete"),
        description: folder.name,
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("library.deleteFolderFailed"));
    } finally {
      actionBusy.value = false;
    }
  }

  function deleteFile(file: LibraryFileSummary) {
    confirm.require({
      header: t("common.delete"),
      message: t("library.deleteFileConfirm", { name: file.filename }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("common.delete"),
      accept: () => {
        void deleteFileConfirmed(file);
      },
    });
  }

  async function deleteFileConfirmed(file: LibraryFileSummary) {
    actionBusy.value = true;

    try {
      await apiClient.deleteLibraryFile(file.file_id);
      await loadTree();

      if (selectedFileId.value === file.file_id) {
        await replaceQuery(selectedFolder.value?.folder_id ?? null, null);
      }
      toast.add({
        color: "success",
        title: t("common.delete"),
        description: file.filename,
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, t("library.deleteFileFailed"));
    } finally {
      actionBusy.value = false;
    }
  }

  function moveExplorerEntry(entry: ExplorerEntry) {
    if (entry.kind === "folder") {
      openMoveFolderDialog(entry.folder);
      return;
    }

    openMoveFileDialog(entry.file);
  }

  function deleteExplorerEntry(entry: ExplorerEntry) {
    if (entry.kind === "folder") {
      void deleteFolder(entry.folder);
      return;
    }

    void deleteFile(entry.file);
  }

  const filteredMoveOptions = computed(() => {
    if (!moveDialog.value) {
      return [];
    }

    return moveOptions.value.filter(
      (option) => !moveDialog.value?.excludedFolderIds.includes(option.value as string),
    );
  });

  const resourceMenuItems = computed<LibraryMenuItem[]>(() => []);

  return {
    actionBusy,
    confirmCreateFolder,
    confirmMove,
    createDialog,
    createFolderBusy,
    deleteExplorerEntry,
    deleteFile,
    deleteFolder,
    filteredMoveOptions,
    handleFileSelection,
    moveDialog,
    moveExplorerEntry,
    openCreateFolderDialog,
    openMoveFileDialog,
    openMoveFolderDialog,
    resourceMenuItems,
    revealPreviewForFile,
    uploadBusy,
  };
}
