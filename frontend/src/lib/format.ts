import { getEffectiveTimeZone } from "./preferences-store";

const DATE_ONLY_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

interface ZonedDateParts {
  year: string;
  month: string;
  day: string;
  hour: string;
  minute: string;
}

function readZonedDateParts(
  value: string | null | undefined,
  includeTime: boolean,
  timeZone: string = getEffectiveTimeZone(),
): ZonedDateParts | null {
  if (!value || DATE_ONLY_PATTERN.test(value)) {
    return null;
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return null;
  }

  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
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

export function formatDate(
  value: string | null | undefined,
  timeZone: string = getEffectiveTimeZone(),
): string {
  if (!value) {
    return "--";
  }

  if (DATE_ONLY_PATTERN.test(value)) {
    return value;
  }

  const parts = readZonedDateParts(value, false, timeZone);
  if (!parts) {
    return value;
  }

  return `${parts.year}-${parts.month}-${parts.day}`;
}

export interface ElapsedOptions {
  /**
   * True only for a nonterminal task that has not finished yet. A terminal task
   * with a missing or invalid finish timestamp must stay unavailable instead of
   * fabricating a total that keeps moving.
   */
  unfinished: boolean;
}

/**
 * Wall-clock elapsed time between task start and finish, or between start and
 * `now` while the task is still unfinished. Pauses and retry waits are real
 * wall time, so they are included; a missing start, an unparsable start, or a
 * terminal task without a valid finish stays `--` instead of a guessed
 * duration. A negative span clamps to zero.
 */
export function formatElapsedDuration(
  startedAt: string | null | undefined,
  finishedAt: string | null | undefined,
  now: Date | number = Date.now(),
  options: ElapsedOptions = { unfinished: false },
): string {
  if (!startedAt) {
    return "--";
  }

  const startMs = Date.parse(startedAt);
  if (Number.isNaN(startMs)) {
    return "--";
  }

  const finishedMs = finishedAt ? Date.parse(finishedAt) : Number.NaN;
  let endMs = finishedMs;
  if (Number.isNaN(finishedMs)) {
    if (!options.unfinished) {
      return "--";
    }
    endMs = typeof now === "number" ? now : now.getTime();
  }
  if (Number.isNaN(endMs)) {
    return "--";
  }

  const totalSeconds = Math.max(0, Math.floor((endMs - startMs) / 1000));
  const days = Math.floor(totalSeconds / 86_400);
  const hours = Math.floor((totalSeconds % 86_400) / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;

  if (days > 0) {
    return `${days}d ${hours}h ${minutes}m`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes}m ${seconds}s`;
  }
  if (minutes > 0) {
    return `${minutes}m ${seconds}s`;
  }
  return `${seconds}s`;
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
