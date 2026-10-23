import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { MonacoEditor } from "@/components/common/monaco-editor";
import { FormField } from "@/components/common/form-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type { SourceConfigInput, Visibility } from "@/services/api";

type SourceSyncStrategy = SourceConfigInput["sync_strategy"];

export interface SourceConfigDraft {
  source_id: string | null;
  source_key: string;
  display_name: string;
  description: string;
  example_queries: string[];
  connection: string;
  database_url: string;
  sync_strategy: SourceSyncStrategy;
  connector_type: "postgres_sql";
  base_query: string;
  batch_size: number;
  visibility: Visibility | null;
}

function emptyDraft(): SourceConfigDraft {
  return {
    source_id: null,
    source_key: "",
    display_name: "",
    description: "",
    example_queries: [],
    connection: "",
    database_url: "",
    sync_strategy: "cursor",
    connector_type: "postgres_sql",
    base_query: "",
    batch_size: 200,
    visibility: null,
  };
}

function stringValue(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

export function parseSourceDraft(value: string): SourceConfigDraft | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }
  const record = parsed as Record<string, unknown>;
  const examples = Array.isArray(record.example_queries)
    ? record.example_queries.filter((entry): entry is string => typeof entry === "string")
    : [];
  const batch = typeof record.batch_size === "number" ? record.batch_size : 200;
  return {
    source_id: typeof record.source_id === "string" ? record.source_id : null,
    source_key: stringValue(record.source_key),
    display_name: stringValue(record.display_name),
    description: stringValue(record.description),
    example_queries: examples,
    connection: stringValue(record.connection),
    database_url: stringValue(record.database_url),
    sync_strategy: record.sync_strategy === "full_scan" ? "full_scan" : "cursor",
    connector_type: "postgres_sql",
    base_query: stringValue(record.base_query),
    batch_size: Number.isFinite(batch) && batch > 0 ? Math.trunc(batch) : 200,
    visibility: record.visibility === "public" || record.visibility === "private"
      ? record.visibility
      : null,
  };
}

export function serializeSourceDraft(draft: SourceConfigDraft): string {
  const payload: SourceConfigInput = {
    source_id: draft.source_id,
    source_key: draft.source_key,
    display_name: draft.display_name || null,
    description: draft.description || null,
    example_queries: draft.example_queries,
    connection: draft.connection,
    database_url: draft.database_url || null,
    sync_strategy: draft.sync_strategy,
    connector_type: draft.connector_type,
    base_query: draft.base_query,
    batch_size: draft.batch_size,
    visibility: draft.visibility,
  };
  return JSON.stringify(payload, null, 2);
}

/**
 * Source configuration editor. The JSON text is the single source of truth
 * (the API accepts it verbatim); the form mode is a typed projection of that
 * text. Invalid JSON is reported and can only be repaired in JSON mode, so a
 * parse failure never silently discards the operator's text.
 */
export function SourceConfigEditor({
  value,
  onChange,
  disabled,
  showFolderName = false,
  folderName = "",
  onFolderNameChange,
}: {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  showFolderName?: boolean;
  folderName?: string;
  onFolderNameChange?: (value: string) => void;
}) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"form" | "json">("form");
  const draft = useMemo(() => parseSourceDraft(value), [value]);

  function update(patch: Partial<SourceConfigDraft>) {
    if (!draft) {
      return;
    }
    onChange(serializeSourceDraft({ ...draft, ...patch }));
  }

  return (
    <div className="grid min-w-0 gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="inline-flex items-center gap-1 rounded-md border p-0.5" role="tablist" aria-label={t("sources.form.modeLabel")}>
          <Button
            type="button"
            role="tab"
            aria-selected={mode === "form"}
            variant={mode === "form" ? "secondary" : "ghost"}
            size="sm"
            onClick={() => setMode("form")}
          >
            {t("sources.form.formMode")}
          </Button>
          <Button
            type="button"
            role="tab"
            aria-selected={mode === "json"}
            variant={mode === "json" ? "secondary" : "ghost"}
            size="sm"
            onClick={() => setMode("json")}
          >
            {t("sources.form.jsonMode")}
          </Button>
        </div>
        {showFolderName ? (
          <div className="min-w-48 flex-1 sm:max-w-64">
            <Input
              value={folderName}
              aria-label={t("library.sourceFolderName")}
              placeholder={t("library.sourceFolderName")}
              data-testid="source-folder-name"
              disabled={disabled}
              onChange={(event) => onFolderNameChange?.(event.target.value)}
            />
          </div>
        ) : null}
      </div>

      {!draft ? (
        <p role="alert" className="rounded-md border border-destructive/40 p-2 text-sm text-destructive">
          {t("library.sourceConfigInvalid")}
        </p>
      ) : null}

      {mode === "json" ? (
        <MonacoEditor
          value={value}
          onChange={onChange}
          language="json"
          readOnly={disabled}
          ariaLabel={t("sources.form.baseQueryLabel")}
          id="source-config-json"
        />
      ) : draft ? (
        <div className="grid gap-3 md:grid-cols-2">
          <FormField label={t("sources.form.sourceKeyLabel")} htmlFor="source-key">
            <Input
              id="source-key"
              value={draft.source_key}
              disabled={disabled}
              onChange={(event) => update({ source_key: event.target.value })}
            />
          </FormField>
          <FormField label={t("sources.form.displayNameLabel")} htmlFor="source-display-name">
            <Input
              id="source-display-name"
              value={draft.display_name}
              disabled={disabled}
              onChange={(event) => update({ display_name: event.target.value })}
            />
          </FormField>
          <FormField label={t("sources.form.connectionLabel")} htmlFor="source-connection">
            <Input
              id="source-connection"
              value={draft.connection}
              disabled={disabled}
              onChange={(event) => update({ connection: event.target.value })}
            />
          </FormField>
          <FormField label={t("sources.form.syncStrategyLabel")} htmlFor="source-sync">
            <Select
              value={draft.sync_strategy}
              disabled={disabled}
              onValueChange={(next) => update({ sync_strategy: next === "full_scan" ? "full_scan" : "cursor" })}
            >
              <SelectTrigger id="source-sync" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="cursor">{t("sources.form.syncCursor")}</SelectItem>
                <SelectItem value="full_scan">{t("sources.form.syncFullScan")}</SelectItem>
              </SelectContent>
            </Select>
          </FormField>
          <FormField label={t("sources.form.batchSizeLabel")} htmlFor="source-batch-size">
            <Input
              id="source-batch-size"
              type="number"
              min={1}
              value={String(draft.batch_size)}
              disabled={disabled}
              onChange={(event) => {
                const next = Number.parseInt(event.target.value, 10);
                update({ batch_size: Number.isFinite(next) && next > 0 ? next : 1 });
              }}
            />
          </FormField>
          <FormField label={t("groups.visibility")} htmlFor="source-visibility">
            <Select
              value={draft.visibility ?? "inherit"}
              disabled={disabled}
              onValueChange={(next) => update({
                visibility: next === "public" || next === "private" ? next : null,
              })}
            >
              <SelectTrigger id="source-visibility" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="inherit">{t("sources.form.visibilityInherit")}</SelectItem>
                <SelectItem value="private">{t("groups.visibilityOptions.private")}</SelectItem>
                <SelectItem value="public">{t("groups.visibilityOptions.public")}</SelectItem>
              </SelectContent>
            </Select>
          </FormField>
          <FormField label={t("sources.form.descriptionLabel")} htmlFor="source-description" className="md:col-span-2">
            <Textarea
              id="source-description"
              value={draft.description}
              disabled={disabled}
              onChange={(event) => update({ description: event.target.value })}
            />
          </FormField>
          <FormField
            label={t("sources.form.exampleQueriesLabel")}
            htmlFor="source-example-queries"
            helper={t("sources.form.exampleQueriesHelper")}
            className="md:col-span-2"
          >
            <Textarea
              id="source-example-queries"
              rows={2}
              value={draft.example_queries.join("\n")}
              disabled={disabled}
              onChange={(event) => update({
                example_queries: event.target.value.split("\n").map((entry) => entry.trim()).filter(Boolean),
              })}
            />
          </FormField>
          <FormField label={t("sources.form.baseQueryLabel")} htmlFor="source-base-query" className="md:col-span-2">
            <Textarea
              id="source-base-query"
              rows={6}
              className="font-mono text-xs"
              value={draft.base_query}
              disabled={disabled}
              onChange={(event) => update({ base_query: event.target.value })}
            />
          </FormField>
        </div>
      ) : null}
    </div>
  );
}
