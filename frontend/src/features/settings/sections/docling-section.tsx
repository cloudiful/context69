import { Info, RefreshCw } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { NumberField } from "@/components/common/number-field";
import { FormField } from "@/components/common/form-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { TextField } from "@/components/common/text-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import { formatTimestamp } from "@/lib/format";
import type { DoclingCleanupTask } from "@/services/api/api-types";
import type { DraftDoclingVlmMode } from "@/utils/settings";

import { TestConnectionButton } from "./test-connection-button";

type CleanupReasonCode = NonNullable<DoclingCleanupTask["reason_code"]>;

export function DoclingSection() {
  const { t } = useTranslation();
  const { page, doclingRemote } = useSettings();
  const draft = page.doclingDraft;

  const vlmModeInfo = {
    disabled: t("settings.docling.vlmModeInfoDisabled"),
    preset: t("settings.docling.vlmModeInfoPreset"),
    custom: t("settings.docling.vlmModeInfoCustom"),
  }[draft.vlm_mode];

  return (
    <SettingsSection title={t("settings.docling.title")}>
      <SettingsGate>
      <div className="grid gap-3">
        <SettingsBlock
          id="settings-connection"
          compact
          title={t("settings.docling.connectionTitle")}
          actions={
            <TestConnectionButton
              testing={page.doclingTesting}
              disabled={!draft.connection.base_url.trim()}
              testId="docling-connection-test"
              label={t("settings.docling.test")}
              onTest={() => void page.testDoclingConnection()}
            />
          }
        >
          <div className="grid max-w-2xl gap-3">
            <TextField
              id="docling-base-url"
              type="url"
              label={t("settings.docling.baseUrl")}
              placeholder="http://127.0.0.1:5001"
              value={draft.connection.base_url}
              onChange={(value) => page.updateDoclingDraft((next) => {
                next.connection.base_url = value;
              })}
            />
            <NumberField
              id="docling-timeout"
              label={t("settings.docling.timeout")}
              min={1}
              value={draft.connection.timeout_secs}
              onChange={(value) => page.updateDoclingDraft((next) => {
                next.connection.timeout_secs = value;
              })}
            />
            <NumberField
              id="docling-poll-interval"
              label={t("settings.docling.pollInterval")}
              min={1}
              value={draft.connection.poll_interval_secs}
              onChange={(value) => page.updateDoclingDraft((next) => {
                next.connection.poll_interval_secs = value;
              })}
            />
            <NumberField
              id="docling-task-timeout"
              label={t("settings.docling.taskTimeout")}
              min={1}
              value={draft.connection.task_timeout_secs}
              onChange={(value) => page.updateDoclingDraft((next) => {
                next.connection.task_timeout_secs = value;
              })}
            />
            <NumberField
              id="docling-max-inflight"
              label={t("settings.docling.maxInflight")}
              helper={t("settings.docling.maxInflightHelper")}
              min={1}
              max={32}
              value={draft.connection.max_inflight}
              onChange={(value) => page.updateDoclingDraft((next) => {
                next.connection.max_inflight = value;
              })}
            />
          </div>
        </SettingsBlock>

        <SettingsBlock id="settings-vlm" compact title={t("settings.docling.vlmTitle")}>
          <div className="flex max-w-2xl items-end gap-2">
            <FormField label={t("settings.docling.vlmModeLabel")} htmlFor="docling-vlm-mode" className="flex-1">
              <Select
                value={draft.vlm_mode}
                onValueChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm_mode = value as DraftDoclingVlmMode;
                })}
              >
                <SelectTrigger id="docling-vlm-mode" aria-label={t("settings.docling.vlmModeLabel")}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="disabled">{t("settings.docling.vlmModeDisabled")}</SelectItem>
                  <SelectItem value="preset">{t("settings.docling.vlmModePreset")}</SelectItem>
                  <SelectItem value="custom">{t("settings.docling.vlmModeCustom")}</SelectItem>
                </SelectContent>
              </Select>
            </FormField>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  data-testid="docling-vlm-mode-info"
                  aria-label={t("settings.docling.vlmModeInfoLabel")}
                >
                  <Info aria-hidden="true" />
                </Button>
              </TooltipTrigger>
              <TooltipContent className="max-w-xs text-xs leading-5">{vlmModeInfo}</TooltipContent>
            </Tooltip>
          </div>

          {draft.vlm_mode === "preset" ? (
            <div className="grid max-w-2xl gap-3">
              <TextField
                id="docling-picture-description-preset"
                label={t("settings.docling.pictureDescriptionPreset")}
                placeholder={t("settings.docling.pictureDescriptionPresetPlaceholder")}
                value={draft.vlm.picture_description_preset}
                onChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm.picture_description_preset = value;
                })}
              />
            </div>
          ) : null}

          {draft.vlm_mode === "custom" ? (
            <div className="grid max-w-2xl gap-3">
              <TextField
                id="docling-openai-base-url"
                type="url"
                label={t("settings.docling.openAiBaseUrl")}
                placeholder="https://openrouter.ai/api/v1"
                value={draft.vlm.openai_base_url}
                onChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm.openai_base_url = value;
                })}
              />
              <TextField
                id="docling-api-key"
                type="password"
                autoComplete="new-password"
                label={t("settings.docling.apiKey")}
                placeholder="sk-..."
                value={draft.vlm.api_key}
                onChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm.api_key = value;
                })}
              />
              <TextField
                id="docling-vlm-pipeline-model"
                label={t("settings.docling.vlmPipelineModel")}
                placeholder="gemini-3-flash"
                value={draft.vlm.vlm_pipeline_model}
                onChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm.vlm_pipeline_model = value;
                })}
              />
              <TextField
                id="docling-picture-description-model"
                label={t("settings.docling.pictureDescriptionModel")}
                placeholder="gpt-4o-mini"
                value={draft.vlm.picture_description_model}
                onChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm.picture_description_model = value;
                })}
              />
              <TextField
                id="docling-code-formula-model"
                label={t("settings.docling.codeFormulaModel")}
                placeholder="gpt-4o-mini"
                value={draft.vlm.code_formula_model}
                onChange={(value) => page.updateDoclingDraft((next) => {
                  next.vlm.code_formula_model = value;
                })}
              />
            </div>
          ) : null}
        </SettingsBlock>

        <RemoteTasksBlock
          loadedOnce={doclingRemote.loadedOnce}
          loading={doclingRemote.loading}
          error={doclingRemote.error}
          errorStatus={doclingRemote.errorStatus}
          items={doclingRemote.items}
          hasMore={doclingRemote.hasMore}
          onRefresh={() => void doclingRemote.refresh()}
          onPrev={doclingRemote.prevPage}
          onNext={doclingRemote.nextPage}
        />

        <CleanupBlock
          items={doclingRemote.cleanup}
          woken={doclingRemote.cleanupWoken}
          cleaning={doclingRemote.cleaning}
          disabled={doclingRemote.errorStatus === 403 || doclingRemote.loading}
          onClean={() => void doclingRemote.cleanStoppedTasks()}
        />
      </div>
      </SettingsGate>
    </SettingsSection>
  );
}

export function doclingStatusLabel(
  status: string,
  t: (key: string) => string,
): string {
  switch (status) {
    case "pending": return t("settings.docling.remoteTasks.statusPending");
    case "cancelling": return t("settings.docling.remoteTasks.statusCancelling");
    case "awaiting_terminal": return t("settings.docling.remoteTasks.statusAwaitingTerminal");
    case "done": return t("settings.docling.remoteTasks.statusDone");
    case "unsupported": return t("settings.docling.remoteTasks.statusUnsupported");
    case "failed": return t("settings.docling.remoteTasks.statusFailed");
    default: return t("settings.docling.remoteTasks.statusUnknown");
  }
}

function cleanupStatusVariant(status: string): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "done": return "default";
    case "failed": return "destructive";
    case "pending": return "secondary";
    case "cancelling":
    case "awaiting_terminal": return "outline";
    default: return "secondary";
  }
}

function cleanupOutcomeLabel(outcome: string | null | undefined, t: (key: string) => string): string {
  switch (outcome) {
    case "success": return t("settings.docling.remoteTasks.outcomeSuccess");
    case "failure": return t("settings.docling.remoteTasks.outcomeFailure");
    case "cancelled": return t("settings.docling.remoteTasks.outcomeCancelled");
    case "not_found": return t("settings.docling.remoteTasks.outcomeNotFound");
    default: return t("settings.docling.remoteTasks.outcomeUnknown");
  }
}

function cleanupReasonLabel(reason: CleanupReasonCode | null | undefined, t: (key: string) => string): string {
  switch (reason) {
    case "missing_submit_endpoint": return t("settings.docling.remoteTasks.reasonMissingSubmitEndpoint");
    case "authentication_failed": return t("settings.docling.remoteTasks.reasonAuthenticationFailed");
    case "deadline_exceeded": return t("settings.docling.remoteTasks.reasonDeadlineExceeded");
    case "remote_not_found": return t("settings.docling.remoteTasks.reasonRemoteNotFound");
    case "awaiting_remote_terminal": return t("settings.docling.remoteTasks.reasonAwaitingRemoteTerminal");
    default: return t("settings.docling.remoteTasks.reasonUnspecified");
  }
}

function cleanupReasonDetail(reason: CleanupReasonCode | null | undefined, t: (key: string) => string): string {
  switch (reason) {
    case "missing_submit_endpoint": return t("settings.docling.remoteTasks.reasonMissingSubmitEndpointDetail");
    case "authentication_failed": return t("settings.docling.remoteTasks.reasonAuthenticationFailedDetail");
    case "deadline_exceeded": return t("settings.docling.remoteTasks.reasonDeadlineExceededDetail");
    case "remote_not_found": return t("settings.docling.remoteTasks.reasonRemoteNotFoundDetail");
    case "awaiting_remote_terminal": return t("settings.docling.remoteTasks.reasonAwaitingRemoteTerminalDetail");
    default: return t("settings.docling.remoteTasks.reasonUnspecifiedDetail");
  }
}

function RemoteTasksBlock({
  loadedOnce,
  loading,
  error,
  errorStatus,
  items,
  hasMore,
  onRefresh,
  onPrev,
  onNext,
}: {
  loadedOnce: boolean;
  loading: boolean;
  error: string | null;
  errorStatus: number | null;
  items: { task_id: string; task_type: string; task_status: string; created_at: string }[];
  hasMore: boolean;
  onRefresh: () => void;
  onPrev: () => void;
  onNext: () => void;
}) {
  const { t } = useTranslation();
  const rows = useMemo(() => items.map((task) => ({
    ...task,
    createdLabel: formatTimestamp(task.created_at),
  })), [items]);
  const denied = errorStatus === 403;

  return (
    <SettingsBlock
      id="settings-docling-remote-tasks"
      compact
      title={t("settings.docling.remoteTasks.title")}
      description={t("settings.docling.remoteTasks.description")}
      actions={
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-testid="docling-remote-refresh"
          disabled={loading}
          onClick={onRefresh}
        >
          <RefreshCw aria-hidden="true" className={loading ? "animate-spin" : undefined} />
          {t("settings.docling.remoteTasks.refresh")}
        </Button>
      }
    >
      {loading && !loadedOnce ? (
        <p data-testid="docling-remote-loading" className="text-sm text-muted-foreground">
          {t("settings.docling.remoteTasks.loading")}
        </p>
      ) : error ? (
        <div data-testid="docling-remote-error" role="alert" className="grid gap-0.5 text-sm">
          <span>
            {denied
              ? t("settings.docling.remoteTasks.denied")
              : `${t("settings.docling.remoteTasks.loadFailed")}: ${error}`}
          </span>
          {errorStatus !== null ? <span className="text-xs text-muted-foreground">{errorStatus}</span> : null}
        </div>
      ) : !loading && loadedOnce && items.length === 0 ? (
        <p data-testid="docling-remote-empty" className="text-sm text-muted-foreground">
          {t("settings.docling.remoteTasks.empty")}
        </p>
      ) : loadedOnce ? (
        <>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.docling.remoteTasks.taskId")}</TableHead>
                <TableHead>{t("settings.docling.remoteTasks.taskType")}</TableHead>
                <TableHead>{t("settings.docling.remoteTasks.taskStatus")}</TableHead>
                <TableHead>{t("settings.docling.remoteTasks.createdAt")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => (
                <TableRow key={row.task_id}>
                  <TableCell className="font-mono text-xs">{row.task_id}</TableCell>
                  <TableCell>{row.task_type}</TableCell>
                  <TableCell>
                    <Badge variant="secondary" className="whitespace-nowrap">{row.task_status}</Badge>
                  </TableCell>
                  <TableCell className="whitespace-nowrap">{row.createdLabel}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-testid="docling-remote-prev"
              onClick={onPrev}
            >
              {t("settings.docling.remoteTasks.previous")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-testid="docling-remote-next"
              disabled={!hasMore}
              onClick={onNext}
            >
              {t("settings.docling.remoteTasks.next")}
            </Button>
          </div>
        </>
      ) : (
        <p data-testid="docling-remote-empty" className="text-sm text-muted-foreground">
          {t("settings.docling.remoteTasks.empty")}
        </p>
      )}
    </SettingsBlock>
  );
}

function CleanupBlock({
  items,
  woken,
  cleaning,
  disabled,
  onClean,
}: {
  items: DoclingCleanupTask[];
  woken: number | null;
  cleaning: boolean;
  disabled: boolean;
  onClean: () => void;
}) {
  const { t } = useTranslation();

  return (
    <SettingsBlock
      title={t("settings.docling.remoteTasks.cleanupTitle")}
      description={t("settings.docling.remoteTasks.cleanupDescription")}
    >
      <div className="grid gap-2">
        <div>
          <Button type="button" disabled={disabled || cleaning} onClick={onClean}>
            {t("settings.docling.remoteTasks.cleanStopped")}
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          {t("settings.docling.remoteTasks.cleanupEligibility")}
        </p>
        {woken !== null ? (
          <p data-testid="docling-cleanup-woken" className="text-sm">
            {t("settings.docling.remoteTasks.cleanupWoken", { count: woken })}
          </p>
        ) : null}
      </div>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("settings.docling.remoteTasks.taskId")}</TableHead>
            <TableHead>{t("settings.docling.remoteTasks.taskStatus")}</TableHead>
            <TableHead>{t("settings.docling.remoteTasks.outcome")}</TableHead>
            <TableHead>{t("settings.docling.remoteTasks.reason")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {items.length === 0 ? (
            <TableRow>
              <TableCell colSpan={4} className="text-sm text-muted-foreground">
                {t("settings.docling.remoteTasks.cleanupEmpty")}
              </TableCell>
            </TableRow>
          ) : (
            items.map((item) => (
              <TableRow key={item.remote_task_id}>
                <TableCell className="font-mono text-xs">{item.remote_task_id}</TableCell>
                <TableCell>
                  <Badge variant={cleanupStatusVariant(item.status)} className="whitespace-nowrap">
                    {doclingStatusLabel(item.status, t)}
                  </Badge>
                </TableCell>
                <TableCell>{cleanupOutcomeLabel(item.outcome, t)}</TableCell>
                <TableCell className="grid gap-0.5">
                  <span>{cleanupReasonLabel(item.reason_code, t)}</span>
                  <span className="text-xs text-muted-foreground">{cleanupReasonDetail(item.reason_code, t)}</span>
                </TableCell>
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </SettingsBlock>
  );
}
