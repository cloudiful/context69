import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { FormField } from "@/components/common/form-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { apiClient } from "@/services/api";

export function TranslationSettingsSection({
  groupPath,
  canManage,
}: {
  groupPath: string;
  canManage: boolean;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [enabled, setEnabled] = useState(false);
  const [targets, setTargets] = useState("");
  const [sourceLocale, setSourceLocale] = useState("");
  const [glossary, setGlossary] = useState("");
  const [saving, setSaving] = useState(false);

  const settingsQuery = useQuery({
    queryKey: ["group-translation", groupPath],
    queryFn: ({ signal }) => apiClient.getGroupTranslationSettings(groupPath, { signal }),
    retry: false,
  });

  useEffect(() => {
    const value = settingsQuery.data;
    if (!value) {
      return;
    }
    setEnabled(value.enabled);
    setTargets(value.default_target_locales.join(", "));
    setSourceLocale(value.source_locale ?? "");
    setGlossary(value.glossary.map((entry) => `${entry.source} = ${entry.target}`).join("\n"));
  }, [settingsQuery.data]);

  const stats = settingsQuery.data
    ? t("groups.translation.stats", {
      queued: settingsQuery.data.queued_count,
      running: settingsQuery.data.running_count,
      succeeded: settingsQuery.data.succeeded_count,
      failed: settingsQuery.data.failed_count,
    })
    : "";

  function parseGlossary() {
    return glossary.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => {
      const separator = line.indexOf("=");
      if (separator < 1 || !line.slice(separator + 1).trim()) {
        throw new Error(t("groups.translation.invalidGlossary"));
      }
      return { source: line.slice(0, separator).trim(), target: line.slice(separator + 1).trim() };
    });
  }

  async function save() {
    setSaving(true);
    try {
      const entries = parseGlossary();
      await apiClient.updateGroupTranslationSettings(groupPath, {
        enabled,
        default_target_locales: targets.split(",").map((value) => value.trim()).filter(Boolean),
        source_locale: sourceLocale.trim() || null,
        glossary: entries,
      });
      await queryClient.invalidateQueries({ queryKey: ["group-translation", groupPath] });
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
    <section className="grid gap-3 rounded-lg border p-3" data-testid="group-translation-settings">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <strong className="text-sm font-medium">{t("groups.translation.title")}</strong>
        <span className="text-xs text-muted-foreground">{stats}</span>
      </div>
      {settingsQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {settingsQuery.error instanceof Error ? settingsQuery.error.message : t("groups.translation.loadFailed")}
        </p>
      ) : null}
      <label className="flex items-center gap-2 text-sm">
        <Switch
          checked={enabled}
          disabled={settingsQuery.isLoading || !canManage}
          onCheckedChange={setEnabled}
          aria-label={t("groups.translation.enabled")}
        />
        {t("groups.translation.enabled")}
      </label>
      <FormField label={t("groups.translation.targets")} htmlFor="group-translation-targets">
        <Input
          id="group-translation-targets"
          value={targets}
          disabled={!canManage}
          placeholder="zh-CN, ja-JP"
          onChange={(event) => setTargets(event.target.value)}
        />
      </FormField>
      <FormField label={t("groups.translation.source")} htmlFor="group-translation-source">
        <Input
          id="group-translation-source"
          value={sourceLocale}
          disabled={!canManage}
          placeholder="en-US"
          onChange={(event) => setSourceLocale(event.target.value)}
        />
      </FormField>
      <FormField label={t("groups.translation.glossary")} htmlFor="group-translation-glossary">
        <Textarea
          id="group-translation-glossary"
          rows={4}
          value={glossary}
          disabled={!canManage}
          placeholder="stock = 股票"
          onChange={(event) => setGlossary(event.target.value)}
        />
      </FormField>
      {canManage ? (
        <div className="flex justify-end">
          <Button
            type="button"
            disabled={settingsQuery.isLoading || saving}
            aria-busy={saving}
            onClick={() => void save()}
          >
            {t("common.save")}
          </Button>
        </div>
      ) : null}
    </section>
  );
}
