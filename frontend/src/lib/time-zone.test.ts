import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "../test-utils/storage";
import {
  AUTOMATIC_TIME_ZONE,
  FALLBACK_TIME_ZONES,
  TIME_ZONE_STORAGE_KEY,
  UTC_TIME_ZONE,
  isValidTimeZone,
  listSupportedTimeZones,
  readStoredTimeZone,
  resolveBrowserTimeZone,
  resolveInitialTimeZone,
} from "./time-zone";

describe("time zone preference foundation", () => {
  beforeEach(() => {
    installMockStorage();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("accepts named IANA zones and UTC but rejects offsets and arbitrary strings", () => {
    expect(isValidTimeZone("Asia/Shanghai")).toBe(true);
    expect(isValidTimeZone(UTC_TIME_ZONE)).toBe(true);
    expect(isValidTimeZone("+08:00")).toBe(false);
    expect(isValidTimeZone("Mars/Olympus")).toBe(false);
    expect(isValidTimeZone("")).toBe(false);
    expect(isValidTimeZone(null)).toBe(false);
  });

  it("initializes from the stored preference and falls back to automatic", () => {
    expect(resolveInitialTimeZone(window.localStorage)).toBe(AUTOMATIC_TIME_ZONE);

    window.localStorage.setItem(TIME_ZONE_STORAGE_KEY, "Asia/Tokyo");
    expect(readStoredTimeZone(window.localStorage)).toBe("Asia/Tokyo");
    expect(resolveInitialTimeZone(window.localStorage)).toBe("Asia/Tokyo");

    window.localStorage.setItem(TIME_ZONE_STORAGE_KEY, "Not/AZone");
    expect(readStoredTimeZone(window.localStorage)).toBeNull();
    expect(resolveInitialTimeZone(window.localStorage)).toBe(AUTOMATIC_TIME_ZONE);
  });

  it("survives missing and throwing storage", () => {
    expect(readStoredTimeZone(null)).toBeNull();
    expect(resolveInitialTimeZone(null)).toBe(AUTOMATIC_TIME_ZONE);

    const storage = installMockStorage();
    storage.getItem = () => {
      throw new DOMException("Storage denied", "SecurityError");
    };
    expect(readStoredTimeZone(storage)).toBeNull();
    expect(resolveInitialTimeZone(storage)).toBe(AUTOMATIC_TIME_ZONE);
  });

  it("treats explicit UTC as a different preference from automatic", () => {
    window.localStorage.setItem(TIME_ZONE_STORAGE_KEY, UTC_TIME_ZONE);
    expect(resolveInitialTimeZone(window.localStorage)).toBe(UTC_TIME_ZONE);
    expect(resolveInitialTimeZone(window.localStorage)).not.toBe(AUTOMATIC_TIME_ZONE);
  });

  it("lists Intl zones with UTC, the browser zone and the stored selection", () => {
    const zones = listSupportedTimeZones("Asia/Shanghai");
    expect(zones).toContain(UTC_TIME_ZONE);
    expect(zones).toContain("Asia/Shanghai");
    expect(zones).toContain(resolveBrowserTimeZone());
    expect(new Set(zones).size).toBe(zones.length);
  });

  it("falls back to the fixed list when supportedValuesOf is missing", () => {
    vi.spyOn(Intl, "supportedValuesOf").mockImplementation(() => {
      throw new TypeError("unsupported");
    });
    const zones = listSupportedTimeZones();
    expect(zones).toContain(UTC_TIME_ZONE);
    for (const zone of FALLBACK_TIME_ZONES) {
      expect(zones).toContain(zone);
    }
  });

  it("resolves an unavailable browser zone to UTC", () => {
    const original = Intl.DateTimeFormat;
    vi.spyOn(Intl, "DateTimeFormat").mockImplementation(((locale?: string, options?: Intl.DateTimeFormatOptions) => {
      if (!options?.timeZone) {
        return { resolvedOptions: () => ({ timeZone: "Invalid/Zone" }) } as Intl.DateTimeFormat;
      }
      return new original(locale, options);
    }) as unknown as typeof Intl.DateTimeFormat);

    expect(resolveBrowserTimeZone()).toBe(UTC_TIME_ZONE);
  });
});
