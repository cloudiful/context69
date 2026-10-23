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

interface TranslationForm {
  enabled: boolean;
  targets: string;
  sourceLocale: string;
  glossary: string;
}

const EMPTY_FORM: TranslationForm = {
  enabled: false,
  targets: "",
  sourceLocale: "",
  glossary: "",
};

export function TranslationSettingsSection({
  groupPath,
  canManage,
}: {
  groupPath: string;
  canManage: boolean;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [form, setForm] = useState<TranslationForm>(EMPTY_FORM);
  const [formGroupPath, setFormGroupPath] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const settingsQuery = useQuery({
    queryKey: ["group-translation", groupPath],
    queryFn: ({ signal }) => apiClient.getGroupTranslationSettings(groupPath, { signal }),
    retry: false,
  });

  const settings = settingsQuery.data;
  // The form belongs to the group it was seeded from. Another group never shows
  // the former snapshot, and a later read of the same group never overwrites
  // the edits made on it.
  const activeForm = formGroupPath === groupPath ? form : EMPTY_FORM;
  const loaded = formGroupPath === groupPath && !!settings;

  useEffect(() => {
    if (!settings || formGroupPath === groupPath) {
      return;
    }
    setForm({
      enabled: settings.enabled,
      targets: settings.default_target_locales.join(", "),
      sourceLocale: settings.source_locale ?? "",
      glossary: settings.glossary.map((entry) => `${entry.source} = ${entry.target}`).join("\n"),
    });
    setFormGroupPath(groupPath);
  }, [formGroupPath, groupPath, settings]);

  const stats = settings
    ? t("groups.translation.stats", {
      queued: settings.queued_count,
      running: settings.running_count,
      succeeded: settings.succeeded_count,
      failed: settings.failed_count,
    })
    : "";

  function parseGlossary() {
    return activeForm.glossary.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => {
      const separator = line.indexOf("=");
      if (separator < 1 || !line.slice(separator + 1).trim()) {
        throw new Error(t("groups.translation.invalidGlossary"));
      }
      return { source: line.slice(0, separator).trim(), target: line.slice(separator + 1).trim() };
    });
  }

  async function save() {
    if (!loaded) {
      return;
    }
    setSaving(true);
    try {
      const entries = parseGlossary();
      await apiClient.updateGroupTranslationSettings(groupPath, {
        enabled: activeForm.enabled,
        default_target_locales: activeForm.targets.split(",").map((value) => value.trim()).filter(Boolean),
        source_locale: activeForm.sourceLocale.trim() || null,
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
        <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">
          <span>{settingsQuery.error instanceof Error ? settingsQuery.error.message : t("groups.translation.loadFailed")}</span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={settingsQuery.isFetching}
            data-testid="group-translation-retry"
            onClick={() => void settingsQuery.refetch()}
          >
            {t("common.retry")}
          </Button>
        </div>
      ) : null}
      <label className="flex items-center gap-2 text-sm">
        <Switch
          checked={activeForm.enabled}
          disabled={!loaded || !canManage}
          onCheckedChange={(checked) => setForm({ ...activeForm, enabled: checked })}
          aria-label={t("groups.translation.enabled")}
        />
        {t("groups.translation.enabled")}
      </label>
      <FormField label={t("groups.translation.targets")} htmlFor="group-translation-targets">
        <Input
          id="group-translation-targets"
          value={activeForm.targets}
          disabled={!loaded || !canManage}
          placeholder="zh-CN, ja-JP"
          onChange={(event) => setForm({ ...activeForm, targets: event.target.value })}
        />
      </FormField>
      <FormField label={t("groups.translation.source")} htmlFor="group-translation-source">
        <Input
          id="group-translation-source"
          value={activeForm.sourceLocale}
          disabled={!loaded || !canManage}
          placeholder="en-US"
          onChange={(event) => setForm({ ...activeForm, sourceLocale: event.target.value })}
        />
      </FormField>
      <FormField label={t("groups.translation.glossary")} htmlFor="group-translation-glossary">
        <Textarea
          id="group-translation-glossary"
          rows={4}
          value={activeForm.glossary}
          disabled={!loaded || !canManage}
          placeholder="stock = 股票"
          onChange={(event) => setForm({ ...activeForm, glossary: event.target.value })}
        />
      </FormField>
      {canManage ? (
        <div className="flex justify-end">
          <Button
            type="button"
            disabled={!loaded || saving}
            aria-busy={saving}
            data-testid="group-translation-save"
            onClick={() => void save()}
          >
            {t("common.save")}
          </Button>
        </div>
      ) : null}
    </section>
  );
}
