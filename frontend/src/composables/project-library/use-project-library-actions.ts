import { computed, ref, toValue, type MaybeRefOrGetter, type Ref } from "vue";
import { useToast } from "@nuxt/ui/composables";
import { useAppConfirm } from "../use-app-confirm";

import { apiClient, type LibraryFileSummary, type LibraryFolderNode, type TaskRef } from "../../services/api";
import type { ExplorerEntry } from "../../types/library";
import { collectDescendantFolderIds } from "../../utils/library-tree";
import { useErrorToast } from "../use-error-toast";
import { createTaskSettler } from "../use-task-settling";

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

interface CreateTextDialogState {
  parentFolderId: string | null;
  parentFolderName: string;
}

interface UseProjectLibraryActionsOptions {
  groupPath: MaybeRefOrGetter<string>;
  loadTree: () => Promise<void>;
  moveOptions: Ref<Array<{ label: string; value: string | null }>>;
  replaceSelection: (folderId: string | null, fileId: string | null) => Promise<void>;
  selectFile: (fileId: string) => Promise<void>;
  selectedFolder: Ref<LibraryFolderNode | null>;
  selectedFileId: Ref<string | null>;
  t: (key: string, params?: Record<string, unknown>) => string;
  updateExpandedForFolder: (folderId: string | null) => void;
  previewDocked: Ref<boolean>;
  previewDialogVisible: Ref<boolean>;
}

export function useProjectLibraryActions({
  groupPath,
  loadTree,
  moveOptions,
  replaceSelection,
  selectFile,
  selectedFolder,
  selectedFileId,
  t,
  updateExpandedForFolder,
  previewDocked,
  previewDialogVisible,
}: UseProjectLibraryActionsOptions) {
  const confirm = useAppConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const createFolderBusy = ref(false);
  const uploadBusy = ref(false);
  const actionBusy = ref(false);
  const retryingFileIds = ref<string[]>([]);
  const unavailableFileIds = ref<string[]>([]);
  const moveDialog = ref<MoveDialogState | null>(null);
  const createDialog = ref<CreateDialogState | null>(null);
  const createTextDialog = ref<CreateTextDialogState | null>(null);
  const settler = createTaskSettler(() => loadTree());

  function notifySettledFailures(results: Array<{ status: string }>, messageKey: string) {
    if (results.some((result) => result.status === "failed")) {
      showErrorToast(null, t(messageKey));
    }
  }

  function openCreateFolderDialog(folder: LibraryFolderNode | null = selectedFolder.value) {
    if (!folder) return;
    createDialog.value = {
      parentFolderId: folder.folder_id ?? null,
      parentFolderName: folder.name,
    };
  }

  function openCreateTextDialog(folder: LibraryFolderNode | null = selectedFolder.value) {
    if (!folder) return;
    createTextDialog.value = {
      parentFolderId: folder.folder_id ?? null,
      parentFolderName: folder.name,
    };
  }

  async function confirmCreateFolder(name: string) {
    if (!createDialog.value) return;
    const parentFolderId = createDialog.value.parentFolderId;
    createFolderBusy.value = true;
    try {
      const folder = await apiClient.createGroupLibraryFolder(toValue(groupPath), {
        parent_folder_id: parentFolderId,
        name,
      });
      createDialog.value = null;
      await loadTree();
      updateExpandedForFolder(parentFolderId);
      updateExpandedForFolder(folder.folder_id ?? null);
      await replaceSelection(folder.folder_id, null);
      toast.add({ color: "success", title: t("library.newFolder"), description: folder.name, duration: 2500 });
    } catch (error) {
      showErrorToast(error, t("library.createFolderFailed"));
    } finally {
      createFolderBusy.value = false;
    }
  }

  async function confirmCreateTextFile(payload: { title: string; content: string }) {
    if (!createTextDialog.value) return;
    createFolderBusy.value = true;
    try {
      const task = await apiClient.upsertGroupLibraryText(toValue(groupPath), {
        title: payload.title,
        content: payload.content,
        external_id: crypto.randomUUID(),
        folder_id: createTextDialog.value.parentFolderId,
      });
      createTextDialog.value = null;
      toast.add({ color: "success", title: t("library.newTextFile"), description: payload.title, duration: 2500 });
      notifySettledFailures(await settler.settle([task]), "library.createTextFileFailed");
    } catch (error) {
      showErrorToast(error, t("library.createTextFileFailed"));
    } finally {
      createFolderBusy.value = false;
    }
  }

  async function handleFileSelection(event: { files?: File[] }) {
    const files = Array.from(event.files ?? []);
    if (files.length === 0) return;
    uploadBusy.value = true;
    try {
      const response = await apiClient.uploadGroupLibraryFiles(
        toValue(groupPath),
        selectedFolder.value?.folder_id ?? null,
        files,
      );
      if (response.files.length > 0) {
        await replaceSelection(response.files[0].folder_id ?? selectedFolder.value?.folder_id ?? null, response.files[0].file_id);
      }
      toast.add({ color: "success", title: t("common.upload"), description: t("library.uploadSuccess"), duration: 2500 });
      notifySettledFailures(await settler.settle(response.tasks ?? []), "library.uploadFailed");
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

  async function retryFile(fileId: string) {
    if (retryingFileIds.value.includes(fileId)) return;
    retryingFileIds.value = [...retryingFileIds.value, fileId];
    try {
      const detail = await apiClient.getGroupLibraryFile(toValue(groupPath), fileId);
      if (!detail.source_available) {
        unavailableFileIds.value = [...new Set([...unavailableFileIds.value, fileId])];
        throw new Error(t("library.sourceMissingMessage"));
      }
      const task = await apiClient.submitTask({
        kind: "retry_file_batch",
        group_path: toValue(groupPath),
        items: [{ file_id: fileId }],
      });
      toast.add({
        color: "success",
        title: t("library.retryAccepted"),
        description: t("library.retryAcceptedMessage"),
        duration: 2500,
      });
      notifySettledFailures(await settler.settle([task]), "library.retryFailed");
    } catch (error) {
      showErrorToast(error, t("library.retryFailed"));
    } finally {
      retryingFileIds.value = retryingFileIds.value.filter((id) => id !== fileId);
    }
  }

  function openMoveFolderDialog(folder: LibraryFolderNode) {
    if (!folder.folder_id) return;
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
    if (!moveDialog.value) return;
    actionBusy.value = true;
    try {
      if (moveDialog.value.kind === "folder") {
        await apiClient.moveGroupLibraryFolder(toValue(groupPath), moveDialog.value.id, { target_folder_id: targetFolderId });
        await loadTree();
        await replaceSelection(targetFolderId, null);
      } else {
        await apiClient.moveGroupLibraryFile(toValue(groupPath), moveDialog.value.id, { target_folder_id: targetFolderId });
        await loadTree();
        await replaceSelection(targetFolderId, moveDialog.value.id);
      }
      toast.add({ color: "success", title: t("common.move"), description: moveDialog.value.name, duration: 2500 });
      moveDialog.value = null;
    } catch (error) {
      showErrorToast(error, t("library.moveFailed"));
    } finally {
      actionBusy.value = false;
    }
  }

  function deleteFolder(folder: LibraryFolderNode) {
    if (!folder.folder_id) return;
    confirm.require({
      header: t("common.delete"),
      message: t("library.deleteFolderConfirm", { name: folder.name }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("common.delete"),
      accept: () => { void deleteFolderConfirmed(folder); },
    });
  }

  async function deleteFolderConfirmed(folder: LibraryFolderNode) {
    actionBusy.value = true;
    try {
      const task = await apiClient.deleteGroupLibraryFolder(toValue(groupPath), folder.folder_id!);
      await replaceSelection(null, null);
      toast.add({ color: "success", title: t("common.delete"), description: folder.name, duration: 2500 });
      notifySettledFailures(await settler.settle([task]), "library.deleteFolderFailed");
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
      accept: () => { void deleteFileConfirmed(file); },
    });
  }

  async function deleteFileConfirmed(file: LibraryFileSummary) {
    actionBusy.value = true;
    try {
      const task = await apiClient.deleteGroupLibraryFile(toValue(groupPath), file.file_id);
      if (selectedFileId.value === file.file_id) {
        await replaceSelection(selectedFolder.value?.folder_id ?? null, null);
      }
      toast.add({ color: "success", title: t("common.delete"), description: file.filename, duration: 2500 });
      notifySettledFailures(await settler.settle([task]), "library.deleteFileFailed");
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
    if (!moveDialog.value) return [];
    return moveOptions.value.filter((option) => !moveDialog.value?.excludedFolderIds.includes(option.value as string));
  });

  return {
    actionBusy,
    confirmCreateFolder,
    confirmCreateTextFile,
    confirmMove,
    createDialog,
    createFolderBusy,
    createTextDialog,
    deleteExplorerEntry,
    deleteFile,
    deleteFolder,
    dispose: settler.dispose,
    filteredMoveOptions,
    handleFileSelection,
    moveDialog,
    moveExplorerEntry,
    openCreateFolderDialog,
    openCreateTextDialog,
    openMoveFileDialog,
    openMoveFolderDialog,
    revealPreviewForFile,
    retryFile,
    retryingFileIds,
    unavailableFileIds,
    uploadBusy,
  };
}
