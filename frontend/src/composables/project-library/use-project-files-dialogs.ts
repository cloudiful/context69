import { computed } from "vue";

type Translate = (key: string, params?: Record<string, unknown>) => string;

export interface MoveDialogFolderOption {
  label: string;
  value: string | null;
}

export interface CreateFolderDialogBinding {
  open: boolean;
  busy?: boolean;
  parentName: string;
}

export interface CreateTextFileDialogBinding {
  open: boolean;
  busy?: boolean;
  parentName: string;
}

export interface MoveDialogBinding {
  open: boolean;
  busy?: boolean;
  title: string;
  description: string;
  currentFolderId?: string | null;
  options: MoveDialogFolderOption[];
}

export interface SourceFolderDialogBinding {
  open: boolean;
  busy: boolean;
  folderName?: string;
  folderNameReadonly?: boolean;
  title: string;
  value: string;
}

export interface ProjectFilesLibraryDialogsBindings {
  createFolder: CreateFolderDialogBinding;
  createTextFile: CreateTextFileDialogBinding;
  move: MoveDialogBinding;
  sourceFolder: SourceFolderDialogBinding;
}

interface CreateDialogState {
  parentFolderName: string;
}

interface MoveDialogState {
  kind: "file" | "folder";
  name: string;
  currentFolderId: string | null;
}

interface LibraryActionDialogs {
  createDialog: CreateDialogState | null;
  createTextDialog: CreateDialogState | null;
  moveDialog: MoveDialogState | null;
  createFolderBusy: boolean;
  actionBusy: boolean;
  filteredMoveOptions: MoveDialogFolderOption[];
  confirmCreateFolder: (name: string) => Promise<void>;
  confirmCreateTextFile: (payload: { title: string; content: string }) => Promise<void>;
  confirmMove: (targetFolderId: string | null) => Promise<void>;
}

interface SourceFolderDialogState {
  open: boolean;
  busy: boolean;
  folderName: string;
  folderId: string | null;
  title: string;
  value: string;
  save: (payload: { folderName: string; value: string }) => Promise<void>;
}

interface Options {
  actions: LibraryActionDialogs;
  sourceFolder: SourceFolderDialogState;
  t: Translate;
}

export function useProjectFilesDialogs({ actions, sourceFolder, t }: Options) {
  const bindings = computed<ProjectFilesLibraryDialogsBindings>(() => ({
    createFolder: {
      open: !!actions.createDialog,
      busy: actions.createFolderBusy,
      parentName: actions.createDialog?.parentFolderName ?? t("library.rootFolder"),
    },
    createTextFile: {
      open: !!actions.createTextDialog,
      busy: actions.createFolderBusy,
      parentName: actions.createTextDialog?.parentFolderName ?? t("library.rootFolder"),
    },
    move: {
      open: !!actions.moveDialog,
      busy: actions.actionBusy,
      title: actions.moveDialog?.kind === "folder"
        ? t("library.moveFolderTitle", { name: actions.moveDialog?.name ?? "" })
        : t("library.moveFileTitle", { name: actions.moveDialog?.name ?? "" }),
      description: actions.moveDialog?.kind === "folder" ? t("library.moveFolderDescription") : t("library.moveFileDescription"),
      currentFolderId: actions.moveDialog?.currentFolderId ?? null,
      options: actions.filteredMoveOptions,
    },
    sourceFolder: {
      open: sourceFolder.open,
      busy: sourceFolder.busy,
      folderName: sourceFolder.folderName,
      folderNameReadonly: !!sourceFolder.folderId,
      title: sourceFolder.title,
      value: sourceFolder.value,
    },
  }));

  function cancelCreateFolder() {
    actions.createDialog = null;
  }

  function confirmCreateFolder(name: string) {
    void actions.confirmCreateFolder(name);
  }

  function cancelCreateTextFile() {
    actions.createTextDialog = null;
  }

  function confirmCreateTextFile(payload: { title: string; content: string }) {
    void actions.confirmCreateTextFile(payload);
  }

  function cancelMove() {
    actions.moveDialog = null;
  }

  function confirmMove(targetFolderId: string | null) {
    void actions.confirmMove(targetFolderId);
  }

  function cancelSourceFolder() {
    sourceFolder.open = false;
  }

  function confirmSourceFolder(payload: { folderName: string; value: string }) {
    void sourceFolder.save(payload);
  }

  function updateSourceFolderValue(value: string) {
    sourceFolder.value = value;
  }

  return {
    bindings,
    cancelCreateFolder,
    confirmCreateFolder,
    cancelCreateTextFile,
    confirmCreateTextFile,
    cancelMove,
    confirmMove,
    cancelSourceFolder,
    confirmSourceFolder,
    updateSourceFolderValue,
  };
}
