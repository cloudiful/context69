import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormField } from "@/components/common/form-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { Visibility } from "@/services/api";

import { useGroupWorkspaceContext } from "./group-workspace";
import { MetadataIndexesSection } from "./metadata-indexes-section";
import { TranslationSettingsSection } from "./translation-settings-section";

export function GroupSettingsPage() {
  const { t } = useTranslation();
  const state = useGroupWorkspaceContext();
  const [name, setName] = useState("");
  const [visibility, setVisibility] = useState<Visibility>("private");

  useEffect(() => {
    setName(state.group?.name ?? "");
    setVisibility(state.group?.visibility ?? "private");
  }, [state.group]);

  const hasChanges = name.trim() !== (state.group?.name ?? "")
    || visibility !== (state.group?.visibility ?? "private");

  return (
    <div className="grid min-w-0 gap-3 overflow-y-auto" data-testid="group-settings">
      <section className="grid gap-3 rounded-lg border p-3">
        <dl className="grid gap-2">
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b pb-2">
            <dt className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{t("groups.groupKey")}</dt>
            <dd className="min-w-0 text-right text-sm font-semibold">{state.group?.group_key || state.groupKey}</dd>
          </div>
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b pb-2">
            <dt className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{t("groups.groupPath")}</dt>
            <dd className="min-w-0 truncate text-right text-sm font-semibold">{state.group?.group_path || state.groupPath}</dd>
          </div>
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b pb-2">
            <dt className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{t("groups.kind")}</dt>
            <dd className="text-right"><Badge variant="outline">{state.group?.kind || "--"}</Badge></dd>
          </div>
          <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-1 border-b pb-2">
            <dt className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{t("groups.visibility")}</dt>
            <dd className="min-w-0">
              {state.canManage ? (
                <Select
                  value={visibility}
                  onValueChange={(next) => setVisibility(next === "public" ? "public" : "private")}
                >
                  <SelectTrigger className="w-32" aria-label={t("groups.visibility")} data-testid="group-visibility">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="private">{t("groups.visibilityOptions.private")}</SelectItem>
                    <SelectItem value="public">{t("groups.visibilityOptions.public")}</SelectItem>
                  </SelectContent>
                </Select>
              ) : (
                <Badge variant="secondary">{state.group?.visibility || "--"}</Badge>
              )}
            </dd>
          </div>
        </dl>

        {state.canManage ? (
          <form
            className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end"
            onSubmit={(event) => {
              event.preventDefault();
              if (!name.trim() || !hasChanges) {
                return;
              }
              state.saveGroup({ name: name.trim(), visibility });
            }}
          >
            <FormField label={t("groups.groupName")} htmlFor="group-settings-name">
              <Input
                id="group-settings-name"
                value={name}
                data-testid="group-settings-name"
                onChange={(event) => setName(event.target.value)}
              />
            </FormField>
            <Button
              type="submit"
              disabled={state.groupBusy || !name.trim() || !hasChanges}
              aria-busy={state.groupBusy}
            >
              {t("common.save")}
            </Button>
          </form>
        ) : null}
      </section>

      <MetadataIndexesSection groupPath={state.groupPath} canManage={state.canManage} />
      <TranslationSettingsSection groupPath={state.groupPath} canManage={state.canManage} />

      <section className="flex flex-wrap items-center gap-2 rounded-lg border p-3">
        {state.canManage ? (
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              if (state.group) {
                state.openMoveGroup(state.group);
              }
            }}
          >
            {t("common.move")}
          </Button>
        ) : null}
        {state.canOwn ? (
          <Button type="button" variant="destructive" onClick={state.requestDeleteGroup}>
            {t("common.delete")}
          </Button>
        ) : null}
      </section>
    </div>
  );
}
