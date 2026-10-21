import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { beforeEach, describe, expect, it } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { installMockStorage } from "../test-utils/storage";
import { LOCALE_STORAGE_KEY } from "../i18n/locale";
import { useUiPreferences } from "./use-ui-preferences";
import { useAppearanceDraft } from "./use-appearance-draft";
import { AUTOMATIC_TIME_ZONE, TIME_ZONE_STORAGE_KEY, setTimeZonePreference } from "../utils/time-zone";

const THEME_STORAGE_KEY = "context69.theme";

function mountDraft() {
  const i18n = createTestI18n("en");
  let draft!: ReturnType<typeof useAppearanceDraft>;

  const Harness = defineComponent({
    setup() {
      draft = useAppearanceDraft();
      return () => h("div");
    },
  });

  const wrapper = mount(Harness, { global: { plugins: [i18n] } });

  return { draft, i18n, wrapper };
}

describe("useAppearanceDraft", () => {
  beforeEach(() => {
    installMockStorage();
    window.localStorage.clear();
    setTimeZonePreference(AUTOMATIC_TIME_ZONE);
    useUiPreferences().setTheme("dark");
    window.localStorage.clear();
  });

  it("starts from the applied globals and is not dirty", () => {
    const { draft } = mountDraft();

    expect(draft.draft.locale).toBe("en");
    expect(draft.draft.theme).toBe("dark");
    expect(draft.draft.timeZone).toBe(AUTOMATIC_TIME_ZONE);
    expect(draft.hasChanges.value).toBe(false);
    expect(draft.dirtyFields.value).toEqual([]);
  });

  it("edits only the local draft and never the applied globals or storage", () => {
    const { draft, i18n } = mountDraft();

    draft.updateLocale("zh-CN");
    draft.updateTheme("light");
    draft.updateTimeZone("Asia/Shanghai");

    expect(draft.draft).toMatchObject({ locale: "zh-CN", theme: "light", timeZone: "Asia/Shanghai" });
    expect(draft.dirtyFields.value).toEqual(["locale", "theme", "timeZone"]);
    expect(draft.hasChanges.value).toBe(true);

    expect(i18n.global.locale.value).toBe("en");
    expect(useUiPreferences().state.theme).toBe("dark");
    expect(useUiPreferences().state.timeZone).toBe(AUTOMATIC_TIME_ZONE);
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBeNull();
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBeNull();
  });

  it("ignores invalid selections", () => {
    const { draft } = mountDraft();

    draft.updateLocale("fr");
    draft.updateLocale(null);
    draft.updateTheme("system");
    draft.updateTimeZone("+08:00");
    draft.updateTimeZone(42);

    expect(draft.draft).toMatchObject({ locale: "en", theme: "dark", timeZone: AUTOMATIC_TIME_ZONE });
    expect(draft.hasChanges.value).toBe(false);
  });

  it("commits the submitted snapshot to the applied globals and storage", () => {
    const { draft, i18n } = mountDraft();

    draft.updateLocale("zh-CN");
    draft.updateTheme("light");
    draft.updateTimeZone("Asia/Shanghai");
    const snapshot = draft.captureSnapshot();

    const result = draft.commit(snapshot);

    expect(result).toEqual({ stored: true, unstoredFields: [] });
    expect(i18n.global.locale.value).toBe("zh-CN");
    expect(useUiPreferences().state.theme).toBe("light");
    expect(useUiPreferences().state.timeZone).toBe("Asia/Shanghai");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBe("Asia/Shanghai");
    expect(draft.hasChanges.value).toBe(false);
  });

  it("keeps the saved baseline old, stays dirty, and applies in memory when a write is denied", () => {
    const { draft } = mountDraft();
    const storage = window.localStorage;
    storage.setItem = () => {
      throw new DOMException("Storage denied", "SecurityError");
    };

    draft.updateLocale("zh-CN");
    draft.updateTheme("light");
    draft.updateTimeZone("Asia/Shanghai");
    const snapshot = draft.captureSnapshot();

    const result = draft.commit(snapshot);

    expect(result.stored).toBe(false);
    expect(result.unstoredFields).toEqual(["locale", "theme", "timeZone"]);
    expect(draft.hasChanges.value).toBe(true);
    expect(draft.saved).toMatchObject({ locale: "en", theme: "dark", timeZone: AUTOMATIC_TIME_ZONE });
  });

  it("advances the baseline to the snapshot without overwriting a newer edit", () => {
    const { draft } = mountDraft();

    draft.updateLocale("zh-CN");
    draft.updateTheme("light");
    draft.updateTimeZone("Asia/Shanghai");
    const snapshot = draft.captureSnapshot();

    draft.updateTimeZone("Europe/Paris");

    const result = draft.commit(snapshot);

    expect(result.stored).toBe(true);
    expect(draft.draft.timeZone).toBe("Europe/Paris");
    expect(draft.saved.timeZone).toBe("Asia/Shanghai");
    expect(draft.dirtyFields.value).toEqual(["timeZone"]);
  });

  it("keeps an older snapshot from rolling back a newer edit in the same field", async () => {
    const { draft } = mountDraft();

    draft.updateTheme("light");
    const snapshot = draft.captureSnapshot();
    draft.updateTheme("dark");
    draft.updateTheme("light");
    await flushPromises();

    draft.commit(snapshot);

    expect(draft.saved.theme).toBe("light");
    expect(draft.hasChanges.value).toBe(false);
  });

  it("keeps a failed commit pending when the draft returns to the saved baseline", () => {
    const { draft } = mountDraft();

    draft.updateTimeZone("Asia/Shanghai");
    draft.commit(draft.captureSnapshot());
    expect(draft.hasChanges.value).toBe(false);

    const storage = window.localStorage;
    storage.setItem = () => {
      throw new DOMException("Storage denied", "SecurityError");
    };

    draft.updateTimeZone("UTC");
    const failed = draft.commit(draft.captureSnapshot());

    expect(failed.stored).toBe(false);
    expect(draft.unresolvedCommit.value).toBe(true);
    expect(draft.saved.timeZone).toBe("Asia/Shanghai");
    expect(useUiPreferences().state.timeZone).toBe("UTC");
    expect(draft.hasChanges.value).toBe(true);

    // Returning the draft to the old baseline must not claim the applied UTC
    // appearance is clean.
    draft.updateTimeZone("Asia/Shanghai");
    expect(draft.dirtyFields.value).toEqual([]);
    expect(draft.hasChanges.value).toBe(true);

    // An explicit Save still applies the old values in memory; a still-denied
    // write keeps the unresolved pending state and the old baseline.
    const retried = draft.commit(draft.captureSnapshot());
    expect(retried.stored).toBe(false);
    expect(useUiPreferences().state.timeZone).toBe("Asia/Shanghai");
    expect(draft.hasChanges.value).toBe(true);

    // Once storage works again, saving the current snapshot clears everything.
    installMockStorage();
    const recovered = draft.commit(draft.captureSnapshot());
    expect(recovered.stored).toBe(true);
    expect(draft.unresolvedCommit.value).toBe(false);
    expect(draft.hasChanges.value).toBe(false);
    expect(draft.saved.timeZone).toBe("Asia/Shanghai");
    expect(window.localStorage.getItem(TIME_ZONE_STORAGE_KEY)).toBe("Asia/Shanghai");
  });
});
