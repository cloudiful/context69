import { useI18n } from "vue-i18n";
import { useToast } from "@nuxt/ui/composables";

import { ApiError } from "../services/api/api-core";

export function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

// Stable, sanitized view of an API/transport error: HTTP status from the
// ApiError wrapper plus a bounded upstream message suitable for a tooltip.
// ApiErrorResponse.details and other operator internals are intentionally
// not exposed; callers should render only `status` and a clipped `message`.
export interface ApiErrorSummary {
  status: number | null;
  message: string;
}

export function summarizeApiError(error: unknown, maxLength = 240): ApiErrorSummary {
  const clip = (value: string) =>
    value.length > maxLength ? `${value.slice(0, maxLength - 1)}…` : value;
  if (error instanceof ApiError) {
    return { status: error.status, message: clip(error.message) };
  }
  if (error instanceof Error && error.message) {
    return { status: null, message: clip(error.message) };
  }
  return { status: null, message: clip(String(error)) };
}

export function useErrorToast() {
  const { t } = useI18n();
  const toast = useToast();

  return (error: unknown, fallback: string) => {
    toast.add({
      color: "error",
      title: t("common.error"),
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  };
}