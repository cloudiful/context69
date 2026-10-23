import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import { Button } from "@/components/ui/button";
import { apiClient } from "@/services/api";
import { formatTimestamp } from "@/lib/format";

import { libraryDependencyLabel, queueStageLabel } from "./queue-helpers";

function statusTone(status: string): StatusTone {
  switch (status) {
    case "queued":
      return "info";
    case "running":
      return "active";
    case "waiting":
      return "warning";
    case "succeeded":
      return "success";
    case "failed":
      return "failure";
    case "cancelled":
      return "neutral";
    default:
      return "neutral";
  }
}

/**
 * Read-only troubleshooting for one task. Everything shown is a fact the server
 * reported at `observed_at`: consistency verdict, dependency gates, and the
 * lowest-ordinal items with their attempts and leases.
 */
export function QueueDiagnosePanel({ taskId }: { taskId: string }) {
  const { t } = useTranslation();
  const diagnose = useQuery({
    queryKey: ["tasks", "diagnose", taskId],
    queryFn: ({ signal }) => apiClient.getTaskDiagnose(taskId, { signal }),
    retry: false,
    staleTime: 5_000,
  });

  if (diagnose.isLoading) {
    return <p className="text-xs text-muted-foreground">{t("common.loading")}</p>;
  }
  if (diagnose.isError || !diagnose.data) {
    return (
      <div className="flex items-center gap-2 text-xs">
        <span className="text-destructive">{t("processingQueue.diagnose.failed")}</span>
        <Button type="button" variant="outline" size="sm" onClick={() => void diagnose.refetch()}>
          {t("processingQueue.diagnose.retry")}
        </Button>
      </div>
    );
  }

  const report = diagnose.data;
  const { consistency } = report;

  return (
    <div className="space-y-3 rounded-md border bg-muted/30 p-3 text-xs">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <StatusBadge tone={consistency.consistent ? "success" : "failure"}>
          {consistency.consistent
            ? t("processingQueue.diagnose.consistent")
            : t("processingQueue.diagnose.inconsistent")}
        </StatusBadge>
        <span className="text-muted-foreground">
          {t("processingQueue.diagnose.observedAt", { time: formatTimestamp(report.observed_at) })}
        </span>
      </div>
      {consistency.mismatches.length > 0
        ? (
            <p className="text-destructive">
              {t("processingQueue.diagnose.mismatches", { fields: consistency.mismatches.join(", ") })}
            </p>
          )
        : null}
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-muted-foreground">
        <span>{t("processingQueue.diagnose.openAttempts", { count: consistency.open_attempt_count })}</span>
        <span>{t("processingQueue.diagnose.nearExhaustion", { count: consistency.near_exhaustion_item_count })}</span>
      </div>

      {report.dependency_gates.length > 0
        ? (
            <section className="space-y-1">
              <h4 className="font-medium">{t("processingQueue.diagnose.dependencyGates")}</h4>
              <ul className="space-y-1">
                {report.dependency_gates.map((gate) => (
                  <li key={gate.dependency_key} className="flex flex-wrap items-center gap-2">
                    <span className="font-medium">{libraryDependencyLabel(t, gate.dependency_key)}</span>
                    <StatusBadge tone={gate.state === "closed" ? "failure" : "neutral"}>
                      {t(`processingQueue.diagnose.gateStates.${gate.state}`, { defaultValue: gate.state })}
                    </StatusBadge>
                    {gate.next_probe_at
                      ? (
                          <span className="text-muted-foreground">
                            {t("processingQueue.diagnose.nextProbe", { time: formatTimestamp(gate.next_probe_at) })}
                          </span>
                        )
                      : null}
                    {gate.last_error
                      ? <span className="w-full truncate text-muted-foreground">{gate.last_error}</span>
                      : null}
                  </li>
                ))}
              </ul>
            </section>
          )
        : null}

      <section className="space-y-1">
        {report.items.length === 0
          ? <p className="text-muted-foreground">{t("processingQueue.diagnose.noItems")}</p>
          : (
              <ul className="space-y-1.5">
                {report.items.map((item) => (
                  <li key={item.item_id} className="space-y-0.5 border-b pb-1.5 last:border-b-0 last:pb-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">
                        {t("processingQueue.diagnose.ordinal", { ordinal: item.ordinal })}
                      </span>
                      <StatusBadge tone={statusTone(item.status)}>{t(`processingQueue.statuses.${item.status}`)}</StatusBadge>
                      <span className="text-muted-foreground">
                        {queueStageLabel(t, item.stage)}
                      </span>
                      <span className="text-muted-foreground">
                        {t("processingQueue.attempts", { count: item.attempt_count })}
                      </span>
                      {item.waiting_reason
                        ? (
                            <span className="text-muted-foreground">
                              {t(`processingQueue.waitingReasons.${item.waiting_reason}`, {
                                defaultValue: item.waiting_reason,
                              })}
                              {item.dependency_key ? `: ${libraryDependencyLabel(t, item.dependency_key)}` : ""}
                            </span>
                          )
                        : null}
                    </div>
                    <div className="flex flex-wrap gap-x-3 gap-y-0.5 text-muted-foreground">
                      {item.next_attempt_at
                        ? (
                            <span>
                              {t("processingQueue.diagnose.nextRetry", { time: formatTimestamp(item.next_attempt_at) })}
                            </span>
                          )
                        : null}
                      {item.lease_expires_at
                        ? (
                            <span>
                              {t("processingQueue.diagnose.leaseExpires", { time: formatTimestamp(item.lease_expires_at) })}
                            </span>
                          )
                        : null}
                      {item.active_attempt
                        ? (
                            <span>
                              {t("processingQueue.diagnose.activeAttempt", { attempt: item.active_attempt.attempt })}
                            </span>
                          )
                        : null}
                    </div>
                    {item.error_message
                      ? (
                          <p className="text-destructive">
                            {t("processingQueue.diagnose.latestError")}: {item.error_message}
                          </p>
                        )
                      : null}
                  </li>
                ))}
              </ul>
            )}
        {report.items_truncated
          ? (
              <p className="text-muted-foreground">
                {t("processingQueue.diagnose.truncated", { count: report.items.length })}
              </p>
            )
          : null}
      </section>
    </div>
  );
}
