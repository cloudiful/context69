import { effectiveTimeZone } from "./time-zone";

const DATE_ONLY_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

interface ZonedDateParts {
  year: string;
  month: string;
  day: string;
  hour: string;
  minute: string;
}

function readZonedDateParts(value: string | null | undefined, includeTime: boolean): ZonedDateParts | null {
  if (!value || DATE_ONLY_PATTERN.test(value)) {
    return null;
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return null;
  }

  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: effectiveTimeZone.value,
    calendar: "gregory",
    numberingSystem: "latn",
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    ...(includeTime ? { hour: "2-digit", minute: "2-digit" } : {}),
  }).formatToParts(date);

  const pick = (type: Intl.DateTimeFormatPartTypes) => parts.find((part) => part.type === type)?.value ?? "";

  return {
    year: pick("year"),
    month: pick("month"),
    day: pick("day"),
    hour: includeTime ? pick("hour") : "00",
    minute: includeTime ? pick("minute") : "00",
  };
}

export function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return "--";
  }

  if (DATE_ONLY_PATTERN.test(value)) {
    return value;
  }

  const parts = readZonedDateParts(value, true);
  if (!parts) {
    return value;
  }

  return `${parts.year}-${parts.month}-${parts.day} ${parts.hour}:${parts.minute}`;
}

export function formatDate(value: string | null | undefined): string {
  if (!value) {
    return "--";
  }

  if (DATE_ONLY_PATTERN.test(value)) {
    return value;
  }

  const parts = readZonedDateParts(value, false);
  if (!parts) {
    return value;
  }

  return `${parts.year}-${parts.month}-${parts.day}`;
}

export function formatBytes(value: number | null | undefined): string {
  if (value == null) {
    return "--";
  }

  if (value < 1024) {
    return `${value} B`;
  }

  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let unitIndex = 0;

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex += 1;
  }

  const digits = size >= 100 ? 0 : size >= 10 ? 1 : 2;
  return `${size.toFixed(digits)} ${units[unitIndex]}`;
}

export function formatScore(value: number): string {
  return value.toFixed(3);
}

export function formatJson(value: Record<string, unknown>): string {
  return JSON.stringify(value, null, 2);
}
