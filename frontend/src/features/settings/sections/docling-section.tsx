import { Info, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { NumberField } from "@/components/common/number-field";
import { FormField } from "@/components/common/form-field";
import { SettingsBlock, SettingsSection } from "@/components/common/settings-section";
import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import { TableColumnHeader } from "@/components/common/table-column-header";
import { TablePagination } from "@/components/common/table-pagination";
import { TextField } from "@/components/common/text-field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useDoclingCleanup, DOCLING_CLEANUP_PAGE_SIZES } from "@/features/settings/use-docling-cleanup";
import { DOCLING_REMOTE_TASKS_PAGE_SIZES } from "@/features/settings/use-docling-remote-tasks";
import { SettingsGate } from "@/features/settings/settings-gate";
import { useSettings } from "@/features/settings/settings-context";
import { formatTimestamp } from "@/lib/format";
import type { DoclingCleanupOutcome, DoclingCleanupStatus, DoclingCleanupTask } from "@/services/api/api-types";
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
          pageSize={doclingRemote.pageSize}
          onPageSize={doclingRemote.setPageSize}
          onRefresh={() => void doclingRemote.refresh()}
          onPrev={doclingRemote.prevPage}
          onNext={doclingRemote.nextPage}
        />

        <CleanupBlock />
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

function cleanupStatusTone(status: string): StatusTone {
  switch (status) {
    case "done": return "success";
    case "failed": return "failure";
    case "pending": return "info";
    case "cancelling": return "active";
    case "awaiting_terminal": return "warning";
    case "unsupported": return "neutral";
    default: return "neutral";
  }
}

function remoteTaskTone(status: string): StatusTone {
  switch (status) {
    case "pending": return "info";
    case "started": return "active";
    case "success": return "success";
    case "partial_success": return "warning";
    case "failure": return "failure";
    case "skipped":
    case "cancelled": return "neutral";
    default: return "neutral";
  }
}

function cleanupOutcomeLabel(outcome: string | null | undefined, t: (key: string) => string): string {
  switch (outcome) {
    case "success": return t("settings.docling.remoteTasks.outcomeSuccess");
    case "partial_success": return t("settings.docling.remoteTasks.outcomePartialSuccess");
    case "failure": return t("settings.docling.remoteTasks.outcomeFailure");
    case "skipped": return t("settings.docling.remoteTasks.outcomeSkipped");
    case "cancelled": return t("settings.docling.remoteTasks.outcomeCancelled");
    case "not_found": return t("settings.docling.remoteTasks.outcomeNotFound");
    default: return "--";
  }
}

function cleanupReasonLabel(reason: CleanupReasonCode | null | undefined, t: (key: string) => string): string {
  switch (reason) {
    case "missing_submit_endpoint": return t("settings.docling.remoteTasks.reasonMissingSubmitEndpoint");
    case "authentication_failed": return t("settings.docling.remoteTasks.reasonAuthenticationFailed");
    case "deadline_exceeded": return t("settings.docling.remoteTasks.reasonDeadlineExceeded");
    case "remote_not_found": return t("settings.docling.remoteTasks.reasonRemoteNotFound");
    case "awaiting_remote_terminal": return t("settings.docling.remoteTasks.reasonAwaitingRemoteTerminal");
    default: return "--";
  }
}

function PageSizeSelect({
  label,
  value,
  options,
  testId,
  onChange,
}: {
  label: string;
  value: number;
  options: readonly number[];
  testId: string;
  onChange: (value: number) => void;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-xs text-muted-foreground">{label}</span>
      <Select value={String(value)} onValueChange={(next) => onChange(Number(next))}>
        <SelectTrigger
          id={`${testId}-trigger`}
          size="sm"
          className="w-20"
          aria-label={label}
          data-testid={testId}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((size) => (
            <SelectItem key={size} value={String(size)}>{size}</SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

function RemoteTasksBlock({
  loadedOnce,
  loading,
  error,
  errorStatus,
  items,
  hasMore,
  pageSize,
  onPageSize,
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
  pageSize: number;
  onPageSize: (value: number) => void;
  onRefresh: () => void;
  onPrev: () => void;
  onNext: () => void;
}) {
  const { t } = useTranslation();
  const denied = errorStatus === 403;

  return (
    <SettingsBlock
      id="settings-docling-remote-tasks"
      compact
      title={t("settings.docling.remoteTasks.title")}
      actions={
        <>
          <Badge variant="secondary">{t("settings.docling.remoteTasks.readOnly")}</Badge>
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
        </>
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
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.taskId")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.taskType")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.taskStatus")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.createdAt")} /></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((row) => (
                <TableRow key={row.task_id}>
                  <TableCell className="font-mono text-xs">{row.task_id}</TableCell>
                  <TableCell>{row.task_type}</TableCell>
                  <TableCell>
                    <StatusBadge tone={remoteTaskTone(row.task_status)} className="whitespace-nowrap">{row.task_status}</StatusBadge>
                  </TableCell>
                  <TableCell className="whitespace-nowrap">{formatTimestamp(row.created_at)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <PageSizeSelect
              label={t("common.itemsPerPage")}
              value={pageSize}
              options={DOCLING_REMOTE_TASKS_PAGE_SIZES}
              testId="docling-remote-page-size"
              onChange={onPageSize}
            />
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

const CLEANUP_STATUS_VALUES: DoclingCleanupStatus[] = [
  "pending",
  "cancelling",
  "awaiting_terminal",
  "done",
  "unsupported",
  "failed",
];

const CLEANUP_OUTCOME_VALUES: DoclingCleanupOutcome[] = [
  "success",
  "partial_success",
  "failure",
  "skipped",
  "cancelled",
  "not_found",
];

function CleanupBlock() {
  const { t } = useTranslation();
  const cleanup = useDoclingCleanup();
  const [confirmOpen, setConfirmOpen] = useState(false);
  const denied = cleanup.errorStatus === 403;
  const allStatuses = "all";
  const allOutcomes = "all";

  return (
    <SettingsBlock
      id="settings-docling-cleanup"
      compact
      title={t("settings.docling.remoteTasks.cleanupTitle")}
      actions={
        <>
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="docling-cleanup-refresh"
            disabled={cleanup.loading}
            onClick={() => void cleanup.refresh()}
          >
            <RefreshCw aria-hidden="true" className={cleanup.loading ? "animate-spin" : undefined} />
            {t("settings.docling.remoteTasks.refresh")}
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={denied || cleanup.cleaning}
            onClick={() => setConfirmOpen(true)}
          >
            {t("settings.docling.remoteTasks.cleanStopped")}
          </Button>
        </>
      }
    >
      <div className="flex flex-wrap items-end gap-2">
        <FormField
          label={t("settings.docling.remoteTasks.statusFilter")}
          htmlFor="docling-cleanup-status"
          className="w-40"
        >
          <Select
            value={cleanup.status ?? allStatuses}
            onValueChange={(value) => cleanup.setStatus(value === allStatuses ? null : value as DoclingCleanupStatus)}
          >
            <SelectTrigger id="docling-cleanup-status">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={allStatuses}>{t("settings.docling.remoteTasks.allStatuses")}</SelectItem>
              {CLEANUP_STATUS_VALUES.map((value) => (
                <SelectItem key={value} value={value}>{doclingStatusLabel(value, t)}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </FormField>
        <FormField
          label={t("settings.docling.remoteTasks.outcome")}
          htmlFor="docling-cleanup-outcome"
          className="w-48"
        >
          <Select
            value={cleanup.outcome ?? allOutcomes}
            onValueChange={(value) => cleanup.setOutcome(value === allOutcomes ? null : value as DoclingCleanupOutcome)}
          >
            <SelectTrigger id="docling-cleanup-outcome">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={allOutcomes}>{t("settings.docling.remoteTasks.allOutcomes")}</SelectItem>
              {CLEANUP_OUTCOME_VALUES.map((value) => (
                <SelectItem key={value} value={value}>{cleanupOutcomeLabel(value, t)}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </FormField>
        <FormField
          label={t("settings.docling.remoteTasks.taskId")}
          htmlFor="docling-cleanup-task-id"
          className="w-56"
        >
          <Input
            id="docling-cleanup-task-id"
            value={cleanup.remoteTaskId}
            placeholder={t("settings.docling.remoteTasks.taskIdPlaceholder")}
            onChange={(event) => cleanup.setRemoteTaskId(event.target.value)}
          />
        </FormField>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={!cleanup.hasFilters}
          onClick={cleanup.resetFilters}
        >
          {t("settings.docling.remoteTasks.clearFilters")}
        </Button>
      </div>

      {cleanup.woken !== null ? (
        <p data-testid="docling-cleanup-woken" className="text-sm">
          {t("settings.docling.remoteTasks.cleanupWoken", { count: cleanup.woken })}
        </p>
      ) : null}

      {cleanup.loading && !cleanup.loadedOnce ? (
        <p data-testid="docling-cleanup-loading" className="text-sm text-muted-foreground">
          {t("settings.docling.remoteTasks.loading")}
        </p>
      ) : cleanup.error ? (
        <div data-testid="docling-cleanup-error" role="alert" className="grid gap-0.5 text-sm">
          <span>
            {denied
              ? t("settings.docling.remoteTasks.denied")
              : `${t("settings.docling.remoteTasks.loadFailed")}: ${cleanup.error}`}
          </span>
          {cleanup.errorStatus !== null ? <span className="text-xs text-muted-foreground">{cleanup.errorStatus}</span> : null}
        </div>
      ) : cleanup.loadedOnce && cleanup.items.length === 0 ? (
        <p data-testid="docling-cleanup-empty" className="text-sm text-muted-foreground">
          {t("settings.docling.remoteTasks.cleanupEmpty")}
        </p>
      ) : cleanup.loadedOnce ? (
        <>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.taskId")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.taskStatus")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.outcome")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.reason")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.createdAt")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.updatedAt")} /></TableHead>
                <TableHead><TableColumnHeader label={t("settings.docling.remoteTasks.finishedAt")} /></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {cleanup.items.map((item) => (
                <TableRow key={item.id}>
                  <TableCell className="font-mono text-xs">{item.remote_task_id}</TableCell>
                  <TableCell>
                    <StatusBadge tone={cleanupStatusTone(item.status)} className="whitespace-nowrap">
                      {doclingStatusLabel(item.status, t)}
                    </StatusBadge>
                  </TableCell>
                  <TableCell className="whitespace-nowrap">{cleanupOutcomeLabel(item.outcome, t)}</TableCell>
                  <TableCell className="whitespace-nowrap">{cleanupReasonLabel(item.reason_code, t)}</TableCell>
                  <TableCell className="whitespace-nowrap text-muted-foreground">{formatTimestamp(item.created_at)}</TableCell>
                  <TableCell className="whitespace-nowrap text-muted-foreground">{formatTimestamp(item.updated_at)}</TableCell>
                  <TableCell className="whitespace-nowrap text-muted-foreground">
                    {item.finished_at ? formatTimestamp(item.finished_at) : "--"}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          {cleanup.pagination ? (
            <TablePagination
              pagination={cleanup.pagination}
              onPage={cleanup.setPage}
              onPageSize={cleanup.setPageSize}
              pageSizeOptions={DOCLING_CLEANUP_PAGE_SIZES}
            />
          ) : null}
        </>
      ) : null}

      <ConfirmDialog
        open={confirmOpen}
        title={t("settings.docling.remoteTasks.cleanupConfirmTitle")}
        message={t("settings.docling.remoteTasks.cleanupConfirmMessage")}
        acceptLabel={t("settings.docling.remoteTasks.cleanStopped")}
        onCancel={() => setConfirmOpen(false)}
        onAccept={() => {
          setConfirmOpen(false);
          void cleanup.wakeCleanup();
        }}
      />
    </SettingsBlock>
  );
}
