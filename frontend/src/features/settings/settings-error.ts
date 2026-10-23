import { ApiError } from "@/services/api/api-core";

export function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

export interface SettingsErrorSummary {
  status: number | null;
  message: string;
}

/**
 * Sanitized view of an API/transport error: the wrapped HTTP status plus the
 * upstream message. Response details and other operator internals stay out of
 * the UI.
 */
export function summarizeSettingsError(error: unknown): SettingsErrorSummary {
  if (error instanceof ApiError) {
    return { status: error.status, message: error.message };
  }
  if (error instanceof Error && error.message) {
    return { status: null, message: error.message };
  }
  return { status: null, message: String(error) };
}
