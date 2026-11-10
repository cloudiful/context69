import { useTranslation } from "react-i18next";

import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import type { GitIndexStatus, GitRepositorySource } from "@/services/api";

function gitIndexStatusTone(status: GitIndexStatus): StatusTone {
  switch (status) {
    case "indexing":
    case "pending":
      return "active";
    case "ready":
      return "success";
    case "stale":
      return "warning";
    case "failed":
      return "failure";
    default:
      return "neutral";
  }
}

export function GitIndexStatusBadge({ status }: { status: GitIndexStatus }) {
  const { t } = useTranslation();
  return (
    <StatusBadge tone={gitIndexStatusTone(status)} title={t(`groups.gitSources.indexStatus.${status}`)}>
      {t(`groups.gitSources.indexStatus.${status}`)}
    </StatusBadge>
  );
}

export function shortSha(value: string | null | undefined, length: number): string {
  return value ? value.slice(0, length) : "--";
}

export function gitSkipTotal(source: GitRepositorySource): number {
  const summary = source.last_skip_summary ?? {};
  return (summary.excluded_directory ?? 0)
    + (summary.binary ?? 0)
    + (summary.minified ?? 0)
    + (summary.source_map ?? 0)
    + (summary.lockfile ?? 0)
    + (summary.secret ?? 0)
    + (summary.too_large ?? 0)
    + (summary.excluded_by_user ?? 0)
    + (summary.not_included ?? 0);
}
