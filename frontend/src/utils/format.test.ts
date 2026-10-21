import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { installMockStorage } from "../test-utils/storage";
import { formatDate, formatTimestamp } from "./format";
import { AUTOMATIC_TIME_ZONE, UTC_TIME_ZONE, setTimeZonePreference } from "./time-zone";

describe("format instant in the selected display zone", () => {
  beforeEach(() => {
    installMockStorage();
    setTimeZonePreference(UTC_TIME_ZONE);
  });

  afterEach(() => {
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
  });

  it("renders UTC instants with a 24-hour clock", () => {
    expect(formatTimestamp("2026-03-01T00:00:00.000Z")).toBe("2026-03-01 00:00");
    expect(formatDate("2026-03-01T00:00:00.000Z")).toBe("2026-03-01");
  });

  it("shifts the calendar date across a positive offset day rollover", () => {
    setTimeZonePreference("Asia/Shanghai");

    expect(formatTimestamp("2026-02-28T20:00:00.000Z")).toBe("2026-03-01 04:00");
    expect(formatDate("2026-02-28T20:00:00.000Z")).toBe("2026-03-01");
  });

  it("keeps a negative offset on the previous calendar day", () => {
    setTimeZonePreference("America/New_York");

    expect(formatTimestamp("2026-03-01T02:00:00.000Z")).toBe("2026-02-28 21:00");
    expect(formatDate("2026-03-01T02:00:00.000Z")).toBe("2026-02-28");
  });

  it("follows daylight saving time instead of a fixed offset", () => {
    setTimeZonePreference("America/New_York");

    expect(formatTimestamp("2026-01-15T17:00:00.000Z")).toBe("2026-01-15 12:00");
    expect(formatTimestamp("2026-07-15T16:00:00.000Z")).toBe("2026-07-15 12:00");
  });

  it("renders half-hour offsets", () => {
    setTimeZonePreference("Asia/Kolkata");

    expect(formatTimestamp("2026-03-01T00:00:00.000Z")).toBe("2026-03-01 05:30");
  });

  it("keeps exact date-only strings unchanged in both formatters", () => {
    setTimeZonePreference("America/Los_Angeles");

    expect(formatDate("2026-03-01")).toBe("2026-03-01");
    expect(formatTimestamp("2026-03-01")).toBe("2026-03-01");
  });

  it("returns a placeholder, or an invalid value unchanged", () => {
    expect(formatTimestamp(null)).toBe("--");
    expect(formatTimestamp(undefined)).toBe("--");
    expect(formatDate("")).toBe("--");
    expect(formatTimestamp("not-a-date")).toBe("not-a-date");
    expect(formatDate("2026-13-45")).toBe("2026-13-45");
  });

  it("reacts to a preference change without a reload", () => {
    const instant = "2026-03-01T00:00:00.000Z";
    expect(formatTimestamp(instant)).toBe("2026-03-01 00:00");

    setTimeZonePreference("Asia/Tokyo");
    expect(formatTimestamp(instant)).toBe("2026-03-01 09:00");
  });
});
