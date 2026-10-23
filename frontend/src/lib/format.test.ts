import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "../test-utils/storage";

describe("neutral time zone and formatting foundation", () => {
  beforeEach(() => {
    vi.resetModules();
    installMockStorage();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("validates IANA zones, UTC and the automatic preference", async () => {
    const { isValidTimeZone, isAutomaticTimeZone } = await import("./time-zone");

    expect(isValidTimeZone("UTC")).toBe(true);
    expect(isValidTimeZone("Europe/Berlin")).toBe(true);
    expect(isValidTimeZone("auto")).toBe(false);
    expect(isValidTimeZone("Mars/Olympus")).toBe(false);
    expect(isValidTimeZone("")).toBe(false);
    expect(isAutomaticTimeZone("auto")).toBe(true);
  });

  it("lists supported time zones with UTC and the stored selection", async () => {
    const { listSupportedTimeZones } = await import("./time-zone");
    const zones = listSupportedTimeZones("Asia/Tokyo");

    expect(zones).toContain("UTC");
    expect(zones).toContain("Asia/Tokyo");
    expect(zones).toEqual([...zones].sort((left, right) => left.localeCompare(right)));
  });

  it("formats timestamps in the selected zone with a 24-hour clock", async () => {
    const { applyTimeZoneSelection } = await import("./preferences-store");
    const { formatTimestamp } = await import("./format");

    applyTimeZoneSelection("Asia/Tokyo");

    expect(formatTimestamp("2024-03-10T06:30:00Z")).toBe("2024-03-10 15:30");
    expect(formatTimestamp("2024-03-10T06:30:00Z")).not.toMatch(/AM|PM/);
  });

  it("keeps a plain date value unshifted", async () => {
    const { applyTimeZoneSelection } = await import("./preferences-store");
    const { formatDate, formatTimestamp } = await import("./format");

    applyTimeZoneSelection("Pacific/Honolulu");

    expect(formatDate("2024-03-10")).toBe("2024-03-10");
    expect(formatTimestamp("2024-03-10")).toBe("2024-03-10");
  });

  it("crosses a DST boundary using the zone rules of the selected zone", async () => {
    const { applyTimeZoneSelection } = await import("./preferences-store");
    const { formatTimestamp } = await import("./format");

    applyTimeZoneSelection("America/New_York");

    expect(formatTimestamp("2024-03-10T06:30:00Z")).toBe("2024-03-10 01:30");
    expect(formatTimestamp("2024-03-10T07:30:00Z")).toBe("2024-03-10 03:30");
  });

  it("returns the placeholder for missing values and the raw value when unparsable", async () => {
    const { formatDate, formatTimestamp } = await import("./format");

    expect(formatTimestamp(null)).toBe("--");
    expect(formatDate(undefined)).toBe("--");
    expect(formatTimestamp("not-a-date")).toBe("not-a-date");
  });

  it("formats byte sizes and scores with the existing precision", async () => {
    const { formatBytes, formatScore, formatJson } = await import("./format");

    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.50 KB");
    expect(formatScore(0.5)).toBe("0.500");
    expect(formatJson({ a: 1 })).toBe("{\n  \"a\": 1\n}");
  });
});
