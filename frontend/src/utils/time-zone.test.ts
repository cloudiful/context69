import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "../test-utils/storage";
import {
  AUTOMATIC_TIME_ZONE,
  FALLBACK_TIME_ZONES,
  TIME_ZONE_STORAGE_KEY,
  UTC_TIME_ZONE,
  browserTimeZone,
  effectiveTimeZone,
  getTimeZonePreference,
  isValidTimeZone,
  listSupportedTimeZones,
  readStoredTimeZone,
  resolveBrowserTimeZone,
  resolveInitialTimeZone,
  setTimeZonePreference,
} from "./time-zone";

function replaceSupportedValuesOf(impl: ((key: string) => string[]) | null): () => void {
  const intl = Intl as unknown as { supportedValuesOf?: (key: string) => string[] };
  const original = intl.supportedValuesOf;

  if (impl) {
    intl.supportedValuesOf = impl;
  } else {
    delete intl.supportedValuesOf;
  }

  return () => {
    if (original) {
      intl.supportedValuesOf = original;
    } else {
      delete intl.supportedValuesOf;
    }
  };
}

function expectedZoneList(selected?: string): string[] {
  const zones = new Set<string>([UTC_TIME_ZONE, ...FALLBACK_TIME_ZONES.filter(isValidTimeZone), browserTimeZone]);
  if (selected && isValidTimeZone(selected)) {
    zones.add(selected);
  }
  return [...zones].sort((left, right) => left.localeCompare(right));
}

describe("time zone preference", () => {
  beforeEach(() => {
    installMockStorage();
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
    window.localStorage.clear();
  });

  afterEach(() => {
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
    window.localStorage.clear();
  });

  it("accepts named IANA zones and UTC but rejects offsets and arbitrary strings", () => {
    expect(isValidTimeZone("UTC")).toBe(true);
    expect(isValidTimeZone("Asia/Shanghai")).toBe(true);
    expect(isValidTimeZone("America/Argentina/Buenos_Aires")).toBe(true);
    expect(isValidTimeZone("Asia/Calcutta")).toBe(true);
    expect(isValidTimeZone("+08:00")).toBe(false);
    expect(isValidTimeZone("GMT+8")).toBe(false);
    expect(isValidTimeZone("EST")).toBe(false);
    expect(isValidTimeZone("Not/AZone")).toBe(false);
    expect(isValidTimeZone("")).toBe(false);
    expect(isValidTimeZone(null)).toBe(false);
    expect(isValidTimeZone(`Asia/${"a".repeat(80)}`)).toBe(false);
  });

  it("initializes from the stored preference and falls back to auto", () => {
    const storage = window.localStorage;

    storage.setItem(TIME_ZONE_STORAGE_KEY, "Asia/Tokyo");
    expect(readStoredTimeZone(storage)).toBe("Asia/Tokyo");
    expect(resolveInitialTimeZone(storage)).toBe("Asia/Tokyo");

    storage.setItem(TIME_ZONE_STORAGE_KEY, AUTOMATIC_TIME_ZONE);
    expect(resolveInitialTimeZone(storage)).toBe(AUTOMATIC_TIME_ZONE);

    storage.setItem(TIME_ZONE_STORAGE_KEY, "Not/AZone");
    expect(readStoredTimeZone(storage)).toBeNull();
    expect(resolveInitialTimeZone(storage)).toBe(AUTOMATIC_TIME_ZONE);
  });

  it("survives missing and throwing storage", () => {
    expect(resolveInitialTimeZone(null)).toBe(AUTOMATIC_TIME_ZONE);
    expect(readStoredTimeZone(null)).toBeNull();

    const throwing = {
      getItem() {
        throw new Error("denied");
      },
      setItem() {
        throw new Error("denied");
      },
    } as unknown as Storage;

    expect(readStoredTimeZone(throwing)).toBeNull();
    expect(resolveInitialTimeZone(throwing)).toBe(AUTOMATIC_TIME_ZONE);
  });

  it("handles a denied storage getter on the live window", () => {
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get() {
        throw new DOMException("Storage denied", "SecurityError");
      },
    });

    try {
      expect(readStoredTimeZone()).toBeNull();
      expect(resolveInitialTimeZone()).toBe(AUTOMATIC_TIME_ZONE);
      expect(setTimeZonePreference("Asia/Tokyo")).toBe(true);
      expect(getTimeZonePreference()).toBe("Asia/Tokyo");
      expect(effectiveTimeZone.value).toBe("Asia/Tokyo");
    } finally {
      installMockStorage();
      setTimeZonePreference(AUTOMATIC_TIME_ZONE);
    }
  });

  it("handles a denied storage read on the live window", () => {
    const storage = installMockStorage();
    storage.getItem = () => {
      throw new DOMException("Storage denied", "SecurityError");
    };

    expect(readStoredTimeZone()).toBeNull();
    expect(resolveInitialTimeZone()).toBe(AUTOMATIC_TIME_ZONE);
    expect(setTimeZonePreference("Asia/Tokyo")).toBe(true);
    expect(getTimeZonePreference()).toBe("Asia/Tokyo");

    installMockStorage();
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
  });

  it("keeps the in-memory selection when the live storage write is denied", () => {
    const storage = installMockStorage();
    storage.setItem = () => {
      throw new DOMException("Storage denied", "SecurityError");
    };

    try {
      expect(() => setTimeZonePreference("Asia/Tokyo")).not.toThrow();
      expect(getTimeZonePreference()).toBe("Asia/Tokyo");
      expect(effectiveTimeZone.value).toBe("Asia/Tokyo");
      expect(storage.length).toBe(0);
    } finally {
      installMockStorage();
      setTimeZonePreference(AUTOMATIC_TIME_ZONE);
    }
  });

  it("persists the explicit selection and keeps auto available", () => {
    expect(getTimeZonePreference()).toBe(AUTOMATIC_TIME_ZONE);
    expect(setTimeZonePreference("Asia/Shanghai")).toBe(true);
    expect(getTimeZonePreference()).toBe("Asia/Shanghai");
    expect(effectiveTimeZone.value).toBe("Asia/Shanghai");
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBe("Asia/Shanghai");

    expect(setTimeZonePreference(AUTOMATIC_TIME_ZONE)).toBe(true);
    expect(getTimeZonePreference()).toBe(AUTOMATIC_TIME_ZONE);
    expect(effectiveTimeZone.value).toBe(browserTimeZone);
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBe(AUTOMATIC_TIME_ZONE);
  });

  it("ignores invalid setter input without activating or persisting it", () => {
    setTimeZonePreference("Asia/Tokyo");

    expect(setTimeZonePreference("+08:00")).toBe(false);
    expect(getTimeZonePreference()).toBe("Asia/Tokyo");
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBe("Asia/Tokyo");
  });

  it("treats explicit UTC as a different preference from automatic", () => {
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
    expect(getTimeZonePreference()).toBe(AUTOMATIC_TIME_ZONE);
    expect(effectiveTimeZone.value).toBe(browserTimeZone);

    setTimeZonePreference(UTC_TIME_ZONE);
    expect(getTimeZonePreference()).toBe(UTC_TIME_ZONE);
    expect(effectiveTimeZone.value).toBe(UTC_TIME_ZONE);
  });

  it("lists Intl zones with UTC, the browser zone, and the selected alias", () => {
    setTimeZonePreference("Asia/Calcutta");
    const zones = listSupportedTimeZones();

    expect(zones).toContain(UTC_TIME_ZONE);
    expect(zones).toContain(browserTimeZone);
    expect(zones).toContain("Asia/Calcutta");
    expect(zones).toEqual([...zones].sort((left, right) => left.localeCompare(right)));
  });

  it("falls back to the fixed list when supportedValuesOf is missing", () => {
    const restore = replaceSupportedValuesOf(null);
    try {
      expect(listSupportedTimeZones()).toEqual(expectedZoneList());
    } finally {
      restore();
    }
  });

  it("falls back to the fixed list when supportedValuesOf throws", () => {
    const restore = replaceSupportedValuesOf(() => {
      throw new Error("unsupported");
    });
    try {
      expect(listSupportedTimeZones()).toEqual(expectedZoneList());
    } finally {
      restore();
    }
  });

  it("resolves an unavailable browser zone to UTC", () => {
    const OriginalDateTimeFormat = Intl.DateTimeFormat;
    const invalid = vi.spyOn(Intl, "DateTimeFormat").mockImplementation(((locales?: string | string[], options?: Intl.DateTimeFormatOptions) => {
      if (locales === undefined && options === undefined) {
        return { resolvedOptions: () => ({ timeZone: "Not/AZone" }) } as Intl.DateTimeFormat;
      }
      return new OriginalDateTimeFormat(locales, options);
    }) as typeof Intl.DateTimeFormat);

    try {
      expect(resolveBrowserTimeZone()).toBe(UTC_TIME_ZONE);
    } finally {
      invalid.mockRestore();
    }

    const throwing = vi.spyOn(Intl, "DateTimeFormat").mockImplementation((() => {
      throw new Error("no Intl");
    }) as unknown as typeof Intl.DateTimeFormat);

    try {
      expect(resolveBrowserTimeZone()).toBe(UTC_TIME_ZONE);
    } finally {
      throwing.mockRestore();
    }
  });
});
