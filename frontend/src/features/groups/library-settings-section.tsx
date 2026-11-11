import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { apiClient, groupLibrarySettingsQueryKey } from "@/services/api";
import type { SourcePolicy } from "@/services/api";

export function LibrarySettingsSection({
  groupPath,
  canManage,
}: {
  groupPath: string;
  canManage: boolean;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<{ groupPath: string; value: SourcePolicy } | null>(null);
  const [saving, setSaving] = useState(false);

  const settingsQuery = useQuery({
    queryKey: groupLibrarySettingsQueryKey(groupPath),
    queryFn: ({ signal }) => apiClient.getGroupLibrarySettings(groupPath, { signal }),
    retry: false,
  });

  const settings = settingsQuery.data;
  const value = draft?.groupPath === groupPath ? draft.value : settings?.default_source_policy;
  const releaseAfterProcessing = value === "release_after_processing";
  const changed = !!settings && value !== settings.default_source_policy;

  async function save() {
    if (!settings || !value || !changed) {
      return;
    }
    setSaving(true);
    try {
      const queryKey = groupLibrarySettingsQueryKey(groupPath);
      const response = await apiClient.updateGroupLibrarySettings(groupPath, { default_source_policy: value });
      await queryClient.cancelQueries({ queryKey, exact: true });
      queryClient.setQueryData(queryKey, response);
      setDraft(null);
      await queryClient.invalidateQueries({ queryKey, exact: true });
      toast.success(t("settings.saveSuccess"), { duration: 2500 });
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("settings.saveFailed"),
        duration: 5000,
      });
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="grid gap-3 rounded-lg border p-3" data-testid="group-library-settings">
      <strong className="text-sm font-medium">{t("groups.librarySettings.title")}</strong>
      {settingsQuery.error ? (
        <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">
          <span>{settingsQuery.error instanceof Error ? settingsQuery.error.message : t("groups.librarySettings.loadFailed")}</span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={settingsQuery.isFetching}
            onClick={() => void settingsQuery.refetch()}
          >
            {t("common.retry")}
          </Button>
        </div>
      ) : null}
      <label className="flex items-start gap-2 text-sm">
        <Checkbox
          checked={releaseAfterProcessing}
          disabled={!settings || !canManage}
          data-testid="group-library-release-source"
          onCheckedChange={(checked) => setDraft({ groupPath, value: checked === true ? "release_after_processing" : "retain" })}
        />
        <span className="grid gap-1">
          <span>{t("groups.librarySettings.releaseAfterProcessing")}</span>
          <span className="text-xs text-muted-foreground">{t("groups.librarySettings.description")}</span>
        </span>
      </label>
      {canManage ? (
        <div className="flex justify-end">
          <Button
            type="button"
            disabled={!settings || !changed || saving}
            aria-busy={saving}
            data-testid="group-library-settings-save"
            onClick={() => void save()}
          >
            {t("common.save")}
          </Button>
        </div>
      ) : null}
    </section>
  );
}
