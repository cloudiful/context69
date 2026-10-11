import { describe, expect, it } from "vitest";

import { QUEUE_STAGE_VALUES, QUEUE_WAITING_REASON_VALUES } from "../composables/queue-helpers";
import { PIPELINE_PROGRESS_STAGES } from "../components/processing-queue/task-item-progress";
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

  // The progress stage vocabulary is a separate list from the persisted stage
  // filter values: adding these to `stages` would make the stage filter offer
  // options no stored task stage ever holds.
  it("covers every progress stage in both locales without drift", () => {
    const expected = [...PIPELINE_PROGRESS_STAGES].sort();
    expect(Object.keys(messages.en.processingQueue.progressStageLabels).sort()).toEqual(expected);
    expect(Object.keys(messages["zh-CN"].processingQueue.progressStageLabels).sort()).toEqual(expected);
    for (const stage of PIPELINE_PROGRESS_STAGES) {
      expect(messages.en.processingQueue.progressStageLabels[stage]).toBeTruthy();
      expect(messages["zh-CN"].processingQueue.progressStageLabels[stage]).toBeTruthy();
    }
  });

  it("keeps the stage filter vocabulary free of progress-only stages", () => {
    const filterStages = new Set<string>(Object.keys(messages.en.processingQueue.stages));
    for (const stage of PIPELINE_PROGRESS_STAGES) {
      // `storage` and `indexing` legitimately exist in both vocabularies; the
      // progress-only stages must never leak into the persisted filter list.
      if (stage === "storage" || stage === "indexing") continue;
      expect(filterStages.has(stage)).toBe(false);
    }
    expect([...QUEUE_STAGE_VALUES].sort()).toEqual(
      Object.keys(messages.en.processingQueue.stages).sort(),
    );
  });

  it("keeps every queue waiting reason in both locales without drift", () => {
    const expected = [...QUEUE_WAITING_REASON_VALUES].sort();
    expect(Object.keys(messages.en.processingQueue.waitingReasons).sort()).toEqual(expected);
    expect(Object.keys(messages["zh-CN"].processingQueue.waitingReasons).sort()).toEqual(expected);
    for (const reason of QUEUE_WAITING_REASON_VALUES) {
      expect(messages.en.processingQueue.waitingReasons[reason]).toBeTruthy();
      expect(messages["zh-CN"].processingQueue.waitingReasons[reason]).toBeTruthy();
    }
  });
});
