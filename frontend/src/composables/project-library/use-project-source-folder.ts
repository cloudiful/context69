import { onBeforeUnmount, ref, toValue, type MaybeRefOrGetter } from "vue";
import { useToast } from "@nuxt/ui/composables";

import { apiClient } from "../../services/api";
import type { FileExplorerEntry } from "../../types/library";
import { useErrorToast } from "../use-error-toast";
import { createTaskSettler } from "../use-task-settling";

interface ProjectSourceFolderOptions {
  groupPath: MaybeRefOrGetter<string>;
  selectedFolder: MaybeRefOrGetter<{ folder_id?: string | null } | null>;
  refreshLibrary: () => Promise<void>;
  t: (key: string) => string;
}

export function useProjectSourceFolder(options: ProjectSourceFolderOptions) {
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const busy = ref(false);
  const open = ref(false);
  const title = ref("");
  const folderId = ref<string | null>(null);
  const folderName = ref("");
  const value = ref("");
  const settler = createTaskSettler(() => options.refreshLibrary());

  function defaultTemplate(name = "") {
    return JSON.stringify({
      source_key: name,
      display_name: "",
      description: "",
      example_queries: [],
      connection: "",
      sync_strategy: "cursor",
      connector_type: "postgres_sql",
      base_query: "",
      batch_size: 200,
    }, null, 2);
  }

  function openCreate() {
    folderId.value = null;
    folderName.value = "";
    title.value = options.t("library.newSourceFolder");
    value.value = defaultTemplate();
    open.value = true;
  }

  async function openEditor(entry: FileExplorerEntry) {
    busy.value = true;
    try {
      const detail = await apiClient.getGroupLibraryFile(toValue(options.groupPath), entry.id);
      folderId.value = detail.folder_id ?? null;
      folderName.value = detail.folder_path.split("/").filter(Boolean).at(-1) ?? "";
      title.value = options.t("library.editSourceConfig");
      value.value = detail.sections[0]?.preview_text || defaultTemplate(folderName.value);
      open.value = true;
    } catch (error) {
      showErrorToast(error, options.t("library.detailLoadFailed"));
    } finally {
      busy.value = false;
    }
  }

  async function save(payload: { folderName: string; value: string }) {
    busy.value = true;
    try {
      const sourceConfig = JSON.parse(payload.value);
      if (folderId.value) {
        await apiClient.updateGroupSourceFolderConfig(toValue(options.groupPath), folderId.value, sourceConfig);
      } else {
        await apiClient.createGroupSourceFolder(toValue(options.groupPath), {
          parent_folder_id: toValue(options.selectedFolder)?.folder_id ?? null,
          folder_name: payload.folderName,
          source_config: sourceConfig,
        });
      }
      open.value = false;
      await options.refreshLibrary();
      toast.add({
        color: "success",
        title: folderId.value ? options.t("common.save") : options.t("library.newSourceFolder"),
        description: payload.folderName || sourceConfig.source_key,
        duration: 2500,
      });
    } catch (error) {
      showErrorToast(error, options.t("common.save"));
    } finally {
      busy.value = false;
    }
  }

  async function sync(id: string | null) {
    if (!id) return;
    try {
      const task = await apiClient.syncGroupSourceFolder(toValue(options.groupPath), id);
      toast.add({ color: "success", title: options.t("sources.sync"), description: options.t("sources.syncing"), duration: 2500 });
      const results = await settler.settle([task]);
      if (results.some((result) => result.status === "failed")) {
        showErrorToast(null, options.t("sources.syncFailed"));
      }
    } catch (error) {
      showErrorToast(error, options.t("sources.syncFailed"));
    }
  }

  onBeforeUnmount(() => settler.dispose());

  return { busy, open, title, folderId, folderName, value, openCreate, openEditor, save, sync, dispose: settler.dispose };
}
