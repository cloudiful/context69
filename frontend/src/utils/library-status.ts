import type { LibraryIngestStatus } from "../services/api";
import { useI18n } from "vue-i18n";

type Translate = (key: string, params?: Record<string, unknown>) => string;

export function libraryStatusSeverity(status: LibraryIngestStatus): "success" | "danger" | "warn" | "secondary" {
  switch (status) {
    case "succeeded":
      return "success";
    case "failed":
      return "danger";
    case "running":
      return "warn";
    default:
      return "secondary";
  }
}

export function libraryStatusLabel(t: Translate, status: LibraryIngestStatus): string {
  return t(`library.status.${status}`);
}

export function createLibraryStatusHelpers() {
  const { t } = useI18n();

  return {
    statusLabel(status: LibraryIngestStatus): string {
      return libraryStatusLabel(t, status);
    },
    statusSeverity: libraryStatusSeverity,
  };
}
