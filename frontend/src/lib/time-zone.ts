export const TIME_ZONE_STORAGE_KEY = "context69.timeZone";
export const AUTOMATIC_TIME_ZONE = "auto";
export const UTC_TIME_ZONE = "UTC";

export const FALLBACK_TIME_ZONES = [
  UTC_TIME_ZONE,
  "Africa/Cairo",
  "Africa/Johannesburg",
  "America/Anchorage",
  "America/Argentina/Buenos_Aires",
  "America/Bogota",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Mexico_City",
  "America/New_York",
  "America/Phoenix",
  "America/Sao_Paulo",
  "America/St_Johns",
  "America/Toronto",
  "Asia/Bangkok",
  "Asia/Dubai",
  "Asia/Hong_Kong",
  "Asia/Kolkata",
  "Asia/Seoul",
  "Asia/Shanghai",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Australia/Adelaide",
  "Australia/Brisbane",
  "Australia/Sydney",
  "Europe/Berlin",
  "Europe/London",
  "Europe/Paris",
  "Pacific/Auckland",
  "Pacific/Honolulu",
] as const;

const MAX_TIME_ZONE_LENGTH = 64;
const NAMED_TIME_ZONE_PATTERN = /^[A-Za-z][A-Za-z0-9_+-]*(?:\/[A-Za-z0-9_+-]+)+$/;

export type TimeZonePreference = typeof AUTOMATIC_TIME_ZONE | string;

export function isAutomaticTimeZone(value: string | null | undefined): value is typeof AUTOMATIC_TIME_ZONE {
  return value === AUTOMATIC_TIME_ZONE;
}

export function isValidTimeZone(value: string | null | undefined): value is string {
  if (!value || value.length > MAX_TIME_ZONE_LENGTH) {
    return false;
  }

  if (value !== UTC_TIME_ZONE && !NAMED_TIME_ZONE_PATTERN.test(value)) {
    return false;
  }

  try {
    new Intl.DateTimeFormat("en-US", { timeZone: value });
    return true;
  } catch {
    return false;
  }
}

export function resolveBrowserTimeZone(): string {
  try {
    const detected = new Intl.DateTimeFormat().resolvedOptions().timeZone;
    return isValidTimeZone(detected) ? detected : UTC_TIME_ZONE;
  } catch {
    return UTC_TIME_ZONE;
  }
}

export function readStoredTimeZone(storage: Storage | null | undefined = getStorage()): string | null {
  if (!storage) {
    return null;
  }

  try {
    const stored = storage.getItem(TIME_ZONE_STORAGE_KEY);
    if (stored === AUTOMATIC_TIME_ZONE || isValidTimeZone(stored)) {
      return stored;
    }
  } catch {
    return null;
  }

  return null;
}

export function resolveInitialTimeZone(storage: Storage | null | undefined = getStorage()): TimeZonePreference {
  return readStoredTimeZone(storage) ?? AUTOMATIC_TIME_ZONE;
}

export function getStorage(): Storage | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    const storage = window.localStorage;
    return storage && typeof storage.getItem === "function" && typeof storage.setItem === "function"
      ? storage
      : null;
  } catch {
    return null;
  }
}

function readIntlTimeZones(): string[] | null {
  const intl = Intl as typeof Intl & {
    supportedValuesOf?: (key: string) => string[];
  };

  if (typeof intl.supportedValuesOf !== "function") {
    return null;
  }

  try {
    const values = intl.supportedValuesOf("timeZone");
    return Array.isArray(values) ? values : null;
  } catch {
    return null;
  }
}

export const browserTimeZone = resolveBrowserTimeZone();

export function listSupportedTimeZones(selected?: string | null): string[] {
  const base = readIntlTimeZones() ?? FALLBACK_TIME_ZONES;
  const zones = new Set<string>([UTC_TIME_ZONE]);

  for (const zone of base) {
    if (isValidTimeZone(zone)) {
      zones.add(zone);
    }
  }

  if (isValidTimeZone(browserTimeZone)) {
    zones.add(browserTimeZone);
  }

  if (isValidTimeZone(selected)) {
    zones.add(selected);
  }

  return [...zones].sort((left, right) => left.localeCompare(right));
}
