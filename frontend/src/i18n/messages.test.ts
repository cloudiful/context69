import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS } from "../features/settings/use-personal-access-tokens";
import {
  PIPELINE_PROGRESS_STAGES,
  QUEUE_DEPENDENCY_VALUES,
  QUEUE_STAGE_VALUES,
  QUEUE_WAITING_REASON_VALUES,
  PROCESSING_STATUS_FILTERS,
  TASK_KIND_VALUES,
} from "../features/processing-queue/queue-helpers";
import { en } from "./messages/en";
import { zhCN } from "./messages/zh-CN";

const messages = { en, "zh-CN": zhCN };

const SOURCE_ROOT = join(__dirname, "..", "..");
const CATALOG_ROOT_KEYS = new Set(Object.keys(messages.en));

function collectSourceCatalogKeys(): Set<string> {
  const keys = new Set<string>();
  const literal = /["'`]([a-z][A-Za-z0-9]*\.[A-Za-z0-9_.\-]+)["'`]/g;
  const templatePrefix = /\bt\(\s*`([A-Za-z0-9_.\-]*\.)\$\{/g;

  function walk(dir: string) {
    for (const entry of readdirSync(dir)) {
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) {
        walk(path);
        continue;
      }
      if (!path.endsWith(".ts") && !path.endsWith(".tsx")) {
        continue;
      }
      const text = readFileSync(path, "utf8");
      for (const match of text.matchAll(literal)) {
        const [root] = match[1].split(".");
        if (CATALOG_ROOT_KEYS.has(root)) {
          keys.add(match[1]);
        }
      }
      for (const match of text.matchAll(templatePrefix)) {
        keys.add(match[1]);
      }
    }
  }

  walk(join(SOURCE_ROOT, "src"));
  walk(join(SOURCE_ROOT, "e2e"));
  return keys;
}

function lookup(catalog: object, key: string): unknown {
  return key.split(".").reduce<unknown>((node, part) => (
    node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined
  ), catalog);
}

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

  // Every key any current source or browser test names must resolve in both
  // catalogs; pruning copy must never leave a dangling reference behind.
  it("resolves every source-referenced catalog key in both locales", () => {
    const referenced = collectSourceCatalogKeys();
    expect(referenced.size).toBeGreaterThan(100);
    for (const key of referenced) {
      const bare = key.endsWith(".") ? key.slice(0, -1) : key;
      const value = lookup(messages.en, bare);
      if (key.endsWith(".") || (value !== null && typeof value === "object")) {
        // A dynamic namespace must exist and stay populated in both locales.
        expect(Object.keys(value as object).length, `en missing ${bare}`).toBeGreaterThan(0);
        expect(Object.keys(lookup(messages["zh-CN"], bare) as object).length, `zh-CN missing ${bare}`)
          .toBeGreaterThan(0);
        continue;
      }
      expect(value, `en missing ${key}`).toBeTypeOf("string");
      expect(lookup(messages["zh-CN"], key), `zh-CN missing ${key}`).toBeTypeOf("string");
    }
  });

  it("labels every offered token lifetime in both locales", () => {
    const keys = PERSONAL_ACCESS_TOKEN_EXPIRY_DAYS.map(
      (days) => `settings.personalAccessTokens.expiry${days}Days`,
    );
    expect(keys).toHaveLength(4);
    for (const key of keys) {
      expect(lookup(messages.en, key), `en missing ${key}`).toBeTypeOf("string");
      expect(lookup(messages["zh-CN"], key), `zh-CN missing ${key}`).toBeTypeOf("string");
    }
  });

  it("keeps every dynamic queue domain populated in both locales", () => {
    const domains = [
      ...PROCESSING_STATUS_FILTERS.map((status) => `processingQueue.statuses.${status}`),
      ...["succeeded"].map((status) => `processingQueue.statuses.${status}`),
      ...TASK_KIND_VALUES.map((kind) => `processingQueue.kinds.${kind}`),
      ...QUEUE_STAGE_VALUES.map((stage) => `processingQueue.stages.${stage}`),
      ...QUEUE_WAITING_REASON_VALUES.map((reason) => `processingQueue.waitingReasons.${reason}`),
      ...QUEUE_DEPENDENCY_VALUES.map((dependency) => `processingQueue.dependencies.${dependency}`),
      ...["open", "half_open", "closed"].map((state) => `processingQueue.diagnose.gateStates.${state}`),
      ...["processing", "completed", "trash"].map((view) => `processingQueue.tabs.${view}`),
    ];
    for (const key of domains) {
      expect(lookup(messages.en, key), `en missing ${key}`).toBeTypeOf("string");
      expect(lookup(messages["zh-CN"], key), `zh-CN missing ${key}`).toBeTypeOf("string");
    }
  });
});
