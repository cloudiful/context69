import { computed, ref, toValue, type MaybeRefOrGetter, type Ref } from "vue";
import { useConfirm } from "primevue/useconfirm";
import { useToast } from "primevue/usetoast";

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

interface CreateTextDialogState {
  parentFolderId: string | null;
  parentFolderName: string;
}

interface UseProjectLibraryActionsOptions {
  groupPath: MaybeRefOrGetter<string>;
  loadTree: () => Promise<void>;
  moveOptions: Ref<Array<{ label: string; value: string | null }>>;
  replaceSelection: (folderId: string | null, fileId: string | null) => Promise<void>;
  schedulePolling: (jobIds: string[]) => void;
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
  schedulePolling,
  selectFile,
  selectedFolder,
  selectedFileId,
  t,
  updateExpandedForFolder,
  previewDocked,
  previewDialogVisible,
}: UseProjectLibraryActionsOptions) {
  const confirm = useConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const createFolderBusy = ref(false);
  const uploadBusy = ref(false);
  const actionBusy = ref(false);
  const retryingFileIds = ref<string[]>([]);
  const moveDialog = ref<MoveDialogState | null>(null);
  const createDialog = ref<CreateDialogState | null>(null);
  const createTextDialog = ref<CreateTextDialogState | null>(null);

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
      toast.add({ severity: "success", summary: t("library.newFolder"), detail: folder.name, life: 2500 });
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
      const response = await apiClient.upsertGroupLibraryText(toValue(groupPath), {
        title: payload.title,
        content: payload.content,
        external_id: crypto.randomUUID(),
        folder_id: createTextDialog.value.parentFolderId,
      });
      createTextDialog.value = null;
      await loadTree();
      const nextFile = response.files[0] ?? null;
      if (nextFile) {
        await replaceSelection(nextFile.folder_id ?? selectedFolder.value?.folder_id ?? null, nextFile.file_id);
      }
      schedulePolling(response.jobs.map((job: { job_id: string }) => job.job_id));
      toast.add({ severity: "success", summary: t("library.newTextFile"), detail: payload.title, life: 2500 });
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
      await loadTree();
      if (response.files.length > 0) {
        await replaceSelection(response.files[0].folder_id ?? selectedFolder.value?.folder_id ?? null, response.files[0].file_id);
      }
      schedulePolling(response.jobs.map((job) => job.job_id));
      toast.add({ severity: "success", summary: t("common.upload"), detail: t("library.uploadSuccess"), life: 2500 });
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
      const job = await apiClient.retryGroupLibraryFile(toValue(groupPath), fileId);
      await loadTree();
      schedulePolling([job.job_id]);
      toast.add({
        severity: "success",
        summary: t("library.retryAccepted"),
        detail: t("library.retryAcceptedMessage"),
        life: 2500,
      });
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
      toast.add({ severity: "success", summary: t("common.move"), detail: moveDialog.value.name, life: 2500 });
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
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => { void deleteFolderConfirmed(folder); },
    });
  }

  async function deleteFolderConfirmed(folder: LibraryFolderNode) {
    actionBusy.value = true;
    try {
      await apiClient.deleteGroupLibraryFolder(toValue(groupPath), folder.folder_id!);
      await loadTree();
      await replaceSelection(null, null);
      toast.add({ severity: "success", summary: t("common.delete"), detail: folder.name, life: 2500 });
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
      icon: "pi pi-exclamation-triangle",
      rejectProps: { label: t("common.cancel"), severity: "secondary", outlined: true },
      acceptProps: { label: t("common.delete"), severity: "danger" },
      accept: () => { void deleteFileConfirmed(file); },
    });
  }

  async function deleteFileConfirmed(file: LibraryFileSummary) {
    actionBusy.value = true;
    try {
      await apiClient.deleteGroupLibraryFile(toValue(groupPath), file.file_id);
      await loadTree();
      if (selectedFileId.value === file.file_id) {
        await replaceSelection(selectedFolder.value?.folder_id ?? null, null);
      }
      toast.add({ severity: "success", summary: t("common.delete"), detail: file.filename, life: 2500 });
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
    uploadBusy,
  };
}
