import { describe, expect, it } from "vitest";

import { QUEUE_STAGE_VALUES } from "../composables/queue-helpers";
import { messages } from "./messages";

function collectLeafKeys(node: object, prefix = ""): string[] {
  return Object.entries(node).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return value !== null && typeof value === "object" ? collectLeafKeys(value, path) : [path];
  });
}

describe("i18n message catalogs", () => {
  it("keeps recursive en / zh-CN key parity", () => {
    const enKeys = collectLeafKeys(messages.en);
    const zhKeys = collectLeafKeys(messages["zh-CN"]);

    expect(enKeys.length).toBeGreaterThan(0);
    expect(zhKeys.length).toBe(enKeys.length);

    const enKeySet = new Set(enKeys);
    const zhKeySet = new Set(zhKeys);

    expect(enKeys.filter((key) => !zhKeySet.has(key))).toEqual([]);
    expect(zhKeys.filter((key) => !enKeySet.has(key))).toEqual([]);
  });

  it("covers every queue stage filter value in both locales without drift", () => {
    const expected = [...QUEUE_STAGE_VALUES].sort();
    expect(Object.keys(messages.en.processingQueue.stages).sort()).toEqual(expected);
    expect(Object.keys(messages["zh-CN"].processingQueue.stages).sort()).toEqual(expected);
    for (const stage of QUEUE_STAGE_VALUES) {
      expect(messages.en.processingQueue.stages[stage]).toBeTruthy();
      expect(messages["zh-CN"].processingQueue.stages[stage]).toBeTruthy();
    }
  });
});
