import { expect, test, type Locator, type Page } from "@playwright/test";

import { ADMIN_USER, QUEUE_ITEMS, WORKSPACE_METADATA, queueTaskFixture, type MetadataIndexFixture } from "./fixtures";
import { installAppTransport } from "./transport";

/**
 * Computed-style contrast gate for the refined themes.
 *
 * Playwright renders the built production preview and the typed local mocks, so
 * there is no live backend or database. Every ratio is measured from the real
 * rendered controls (and, for the one control with no product callsite, a
 * test-only element): the in-page bridge normalizes any CSS color through a
 * canvas and composites alpha against the actual ancestor surfaces, so OKLCH,
 * `color()` and translucent tokens are resolved rather than string-compared.
 * Decorative separators and background-only selection tints are intentionally
 * not asserted; only readable text and essential control boundaries are.
 */

const TONES = ["neutral", "active", "info", "success", "warning", "failure"] as const;

const STATUS_TASKS = [
  queueTaskFixture({ task_id: "tone-queued", status: "queued", file_name: "queued.pdf" }),
  queueTaskFixture({ task_id: "tone-running", status: "running", file_name: "running.pdf" }),
  queueTaskFixture({ task_id: "tone-waiting", status: "waiting", file_name: "waiting.pdf" }),
  queueTaskFixture({ task_id: "tone-succeeded", status: "succeeded", file_name: "succeeded.pdf" }),
  queueTaskFixture({ task_id: "tone-failed", status: "failed", file_name: "failed.pdf", error_summary: "boom" }),
  queueTaskFixture({ task_id: "tone-cancelled", status: "cancelled", file_name: "cancelled.pdf" }),
];

type Theme = "light" | "dark";
type ContrastRole = "text" | "border" | "ring" | "fillTrack" | "pair";

interface ContrastResult {
  ratio: number;
  foreground: string;
  background: string;
}

const INSTALL_CONTRAST_BRIDGE = () => {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) {
    throw new Error("2d canvas unavailable for contrast measurement");
  }

  // Transitions would otherwise make an immediate measurement catch the start
  // value (mid-interpolation) of a hover/focus change; the final colors are what
  // the thresholds apply to.
  const frozen = document.createElement("style");
  frozen.textContent = "*,*::before,*::after{transition-duration:0s!important;animation-duration:0s!important}";
  const attachFrozen = () => {
    document.documentElement.appendChild(frozen);
  };
  if (document.head) {
    attachFrozen();
  } else {
    document.addEventListener("DOMContentLoaded", attachFrozen, { once: true });
  }

  interface Rgb {
    r: number;
    g: number;
    b: number;
    a: number;
  }

  function parseColor(css: string): Rgb {
    ctx!.fillStyle = "#ff00ff";
    ctx!.fillStyle = css;
    ctx!.clearRect(0, 0, 1, 1);
    ctx!.fillRect(0, 0, 1, 1);
    const data = ctx!.getImageData(0, 0, 1, 1).data;
    return { r: data[0], g: data[1], b: data[2], a: data[3] / 255 };
  }

  function hex(color: Rgb): string {
    const part = (value: number) => Math.round(value).toString(16).padStart(2, "0");
    return `#${part(color.r)}${part(color.g)}${part(color.b)}`;
  }

  function channel(value: number): number {
    const c = value / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }

  function luminance(color: Rgb): number {
    return 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
  }

  function contrast(a: Rgb, b: Rgb): number {
    const la = luminance(a);
    const lb = luminance(b);
    const hi = Math.max(la, lb);
    const lo = Math.min(la, lb);
    return (hi + 0.05) / (lo + 0.05);
  }

  function over(top: Rgb, bottom: Rgb): Rgb {
    return {
      r: top.r * top.a + bottom.r * (1 - top.a),
      g: top.g * top.a + bottom.g * (1 - top.a),
      b: top.b * top.a + bottom.b * (1 - top.a),
      a: 1,
    };
  }

  function effectiveBackground(node: Element): Rgb {
    const layers: Rgb[] = [];
    let current: Element | null = node;
    while (current) {
      const background = parseColor(getComputedStyle(current).backgroundColor);
      if (background.a > 0) {
        layers.push(background);
      }
      if (background.a >= 0.999) {
        break;
      }
      current = current.parentElement;
    }
    let base: Rgb = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = layers.length - 1; i >= 0; i -= 1) {
      base = over(layers[i], base);
    }
    return base;
  }

  function shadowColors(shadow: string): Rgb[] {
    const matches = shadow.match(/rgba?\([^)]*\)|oklch\([^)]*\)|color\([^)]*\)|#[0-9a-fA-F]{3,8}/g) ?? [];
    return matches.map(parseColor).filter((color) => color.a > 0);
  }

  function measure(el: Element, role: string, childSelector?: string): ContrastResult {
    const target = childSelector ? el.querySelector(childSelector) ?? el : el;
    const style = getComputedStyle(el);
    let foreground: Rgb;
    let background: Rgb;
    if (role === "fillTrack") {
      foreground = parseColor(getComputedStyle(target).backgroundColor);
      background = effectiveBackground(el);
    } else if (role === "pair") {
      foreground = effectiveBackground(target);
      background = effectiveBackground(el);
    } else if (role === "ring") {
      const ring = shadowColors(style.boxShadow)[0];
      if (!ring) {
        throw new Error(`no visible box shadow to measure on ${el.tagName.toLowerCase()}`);
      }
      foreground = ring;
      background = effectiveBackground(el);
    } else if (role === "border") {
      foreground = parseColor(style.borderTopColor);
      background = effectiveBackground(el);
    } else {
      foreground = parseColor(style.color);
      background = effectiveBackground(el);
    }
    // A translucent foreground (for example a `/60` text color) must be
    // composited over its real surface before luminance, otherwise the alpha is
    // discarded and the measured ratio is falsely high.
    const composited = over(foreground, background);
    return {
      ratio: contrast(composited, background),
      foreground: hex(composited),
      background: hex(background),
    };
  }

  function compare(first: Element, second: Element): ContrastResult {
    const firstSurface = effectiveBackground(first);
    const secondSurface = effectiveBackground(second);
    return {
      ratio: contrast(firstSurface, secondSurface),
      foreground: hex(firstSurface),
      background: hex(secondSurface),
    };
  }

  (window as unknown as { __themeContrast: { measure: typeof measure; compare: typeof compare } }).__themeContrast = {
    measure,
    compare,
  };
};

async function measureLocator(
  locator: Locator,
  role: ContrastRole,
  childSelector?: string,
): Promise<ContrastResult> {
  return locator.first().evaluate<ContrastResult, { role: ContrastRole; child?: string }>(
    (el, args) => {
      const bridge = (window as unknown as {
        __themeContrast: { measure: (element: Element, role: string, child?: string) => ContrastResult };
      }).__themeContrast;
      return bridge.measure(el, args.role, args.child);
    },
    { role, child: childSelector },
  );
}

async function expectContrast(
  locator: Locator,
  role: ContrastRole,
  min: number,
  label: string,
  childSelector?: string,
): Promise<void> {
  const result = await measureLocator(locator, role, childSelector);
  expect(result.ratio, `${label} (${JSON.stringify(result)})`).toBeGreaterThanOrEqual(min);
}

/** Rendered gap between two surfaces; decorative separation is design judgement, not a WCAG claim. */
async function compareSurfaces(
  page: Page,
  firstSelector: string,
  secondSelector: string,
  label: string,
): Promise<ContrastResult> {
  const result = await page.evaluate(
    (args) => {
      const bridge = (window as unknown as {
        __themeContrast: {
          compare: (first: Element, second: Element) => ContrastResult;
        };
      }).__themeContrast;
      const first = document.querySelector(args.firstSelector);
      const second = document.querySelector(args.secondSelector);
      if (!first || !second) {
        throw new Error(`${args.label}: missing rendered surface`);
      }
      return bridge.compare(first, second);
    },
    { firstSelector, secondSelector, label },
  );
  expect(result.foreground, `${label} surfaces must not be identical`).not.toBe(result.background);
  return result;
}

async function surfaceColor(page: Page, selector: string, label: string): Promise<string> {
  const result = await page.evaluate(
    (args) => {
      const bridge = (window as unknown as {
        __themeContrast: {
          compare: (first: Element, second: Element) => ContrastResult;
        };
      }).__themeContrast;
      const element = document.querySelector(args.selector);
      if (!element) {
        throw new Error(`${args.label}: missing rendered element`);
      }
      return bridge.compare(element, element).foreground;
    },
    { selector, label },
  );
  return result;
}

function hexHue(hex: string): number {
  const red = Number.parseInt(hex.slice(1, 3), 16) / 255;
  const green = Number.parseInt(hex.slice(3, 5), 16) / 255;
  const blue = Number.parseInt(hex.slice(5, 7), 16) / 255;
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const delta = max - min;
  if (delta === 0) {
    return 0;
  }
  let hue: number;
  if (max === red) {
    hue = ((green - blue) / delta) % 6;
  } else if (max === green) {
    hue = (blue - red) / delta + 2;
  } else {
    hue = (red - green) / delta + 4;
  }
  return (hue * 60 + 360) % 360;
}

function hueDistance(first: number, second: number): number {
  const delta = Math.abs(first - second) % 360;
  return delta > 180 ? 360 - delta : delta;
}

async function focusControl(locator: Locator): Promise<void> {
  await locator.first().focus();
}

/** Open a library folder by its name button (not the expand chevron). */
async function openFolder(page: Page, name: string): Promise<void> {
  const row = page.getByTestId("library-entry-row").filter({ hasText: name }).first();
  await row.locator('button:not([aria-expanded]):not([role="checkbox"]):not([aria-label])').first().click();
}

/**
 * Keyboard modality is what makes a button match `:focus-visible`; the shadow
 * is read in the same evaluation as the Tab so the focus and the style recalc
 * cannot drift apart.
 */
async function measureRingByKeyboard(page: Page, locator: Locator): Promise<ContrastResult> {
  await locator.first().evaluate((el) => el.setAttribute("data-contrast-target", "true"));
  for (let attempt = 0; attempt < 200; attempt += 1) {
    await page.keyboard.press("Tab");
    const result = await page.evaluate(() => {
      const el = document.activeElement;
      if (!el || el.getAttribute("data-contrast-target") !== "true") {
        return null;
      }
      const bridge = (window as unknown as {
        __themeContrast: { measure: (element: Element, role: string, child?: string) => ContrastResult };
      }).__themeContrast;
      return bridge.measure(el, "ring");
    });
    if (result) {
      return result;
    }
  }
  throw new Error("could not reach the target control with the keyboard");
}

async function runContrastSuite(page: Page, theme: Theme): Promise<void> {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.addInitScript(INSTALL_CONTRAST_BRIDGE);
  await page.addInitScript((value) => {
    window.localStorage.setItem("context69.theme", value);
    window.localStorage.setItem("context69.locale", "en");
  }, theme);

  const transport = await installAppTransport(page, {
    user: ADMIN_USER,
    settings: true,
    workspace: {},
    queue: { processing: STATUS_TASKS, items: { "tone-queued": QUEUE_ITEMS } },
  });

  // Processing queue: six status tones, body/muted text, progress fill, outline button.
  await page.goto("/processing-queue");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.getByTestId("queue-task-row")).toHaveCount(STATUS_TASKS.length);

  const successSurface = await surfaceColor(
    page,
    '[data-slot="badge"][data-tone="success"]',
    `${theme}: success badge`,
  );
  const queueSurface = await compareSurfaces(
    page,
    "body",
    '[data-testid="processing-queue-table"]',
    `${theme}: canvas vs queue surface`,
  );
  expect(
    queueSurface.ratio,
    `${theme}: queue surface must rise above the canvas (${JSON.stringify(queueSurface)})`,
  ).toBeGreaterThanOrEqual(1.06);
  const tableHeaderSurface = await compareSurfaces(
    page,
    '[data-testid="processing-queue-table"]',
    '[data-slot="table-header"]',
    `${theme}: queue surface vs table header`,
  );
  expect(
    tableHeaderSurface.ratio,
    `${theme}: table header must separate from its surface (${JSON.stringify(tableHeaderSurface)})`,
  ).toBeGreaterThanOrEqual(1.08);
  const sidebarSurface = await compareSurfaces(
    page,
    "body",
    '[data-slot="sidebar-inner"]',
    `${theme}: canvas vs sidebar`,
  );
  expect(
    sidebarSurface.ratio,
    `${theme}: sidebar must separate from the canvas (${JSON.stringify(sidebarSurface)})`,
  ).toBeGreaterThanOrEqual(1.04);
  const selectedSurface = await compareSurfaces(
    page,
    '[data-slot="sidebar-inner"]',
    '[data-slot="sidebar-menu-button"][data-active="true"]',
    `${theme}: sidebar vs selected navigation`,
  );
  expect(
    selectedSurface.ratio,
    `${theme}: selected navigation must separate from the sidebar (${JSON.stringify(selectedSurface)})`,
  ).toBeGreaterThanOrEqual(1.08);

  for (const tone of TONES) {
    await expectContrast(
      page.locator(`[data-slot="badge"][data-tone="${tone}"]`),
      "text",
      4.5,
      `${theme}: ${tone} status label on its surface`,
    );
  }

  await expectContrast(
    page.locator('[data-testid="queue-task"] span').first(),
    "text",
    4.5,
    `${theme}: row body text`,
  );
  await expectContrast(
    page.locator('[data-testid="queue-progress"] .font-mono').first(),
    "text",
    4.5,
    `${theme}: muted progress text`,
  );
  const inactiveTab = page.getByRole("tab", { name: "Completed" });
  const opaqueTab = await measureLocator(inactiveTab, "text");
  expect(opaqueTab.ratio, `${theme}: inactive tab label (${JSON.stringify(opaqueTab)})`).toBeGreaterThanOrEqual(4.5);
  await inactiveTab.evaluate((el) => {
    el.style.setProperty("color", "color-mix(in oklab, var(--foreground) 60%, transparent)");
  });
  const translucentTab = await measureLocator(inactiveTab, "text");
  await inactiveTab.evaluate((el) => el.style.removeProperty("color"));
  // An alpha-ignoring reader would report the full-strength foreground here, i.e.
  // a ratio above the opaque sample; the helper must composite the alpha instead.
  expect(
    translucentTab.ratio,
    `${theme}: translucent tab foreground must be composited (${JSON.stringify(translucentTab)})`,
  ).toBeLessThan(opaqueTab.ratio);
  if (theme === "light") {
    expect(
      translucentTab.ratio,
      `${theme}: the prior light translucent tab foreground must be rejected (${JSON.stringify(translucentTab)})`,
    ).toBeLessThan(4.5);
  }

  await page.getByRole("button", { name: "Expand task items" }).first().click();
  const progress = page.locator('[role="progressbar"]').first();
  await expect(progress).toBeVisible();
  await expectContrast(progress, "fillTrack", 3, `${theme}: progress fill vs track`, ":scope > div");

  const refresh = page.getByRole("button", { name: "Refresh queue" });
  await expectContrast(refresh, "text", 4.5, `${theme}: outline button label`);
  await refresh.hover();
  await expectContrast(refresh, "text", 4.5, `${theme}: outline button label (hover)`);
  await page.mouse.move(0, 0);

  // Settings runtime: input boundary, focus ring, switches, default button, invalid input.
  await page.goto("/settings/runtime");
  await expect(page.getByRole("heading", { name: "Runtime" })).toBeVisible();

  const input = page.locator("#runtime-embedding-base-url");
  await expectContrast(input, "border", 3, `${theme}: input boundary`);
  await focusControl(input);
  await expectContrast(input, "ring", 3, `${theme}: input focus ring`);

  const switches = page.locator('[data-slot="switch"]');
  const switchCount = await switches.count();
  expect(switchCount).toBeGreaterThan(1);
  for (let index = 0; index < switchCount; index += 1) {
    await expectContrast(
      switches.nth(index),
      "pair",
      3,
      `${theme}: switch indicator ${index}`,
      '[data-slot="switch-thumb"]',
    );
  }

  await page.locator("#runtime-embedding-model").fill("embed-contrast");
  const save = page.getByTestId("settings-save");
  await expect(save).toBeEnabled();
  const primarySurface = await surfaceColor(page, '[data-testid="settings-save"]', `${theme}: primary action`);
  expect(
    hueDistance(hexHue(primarySurface), hexHue(successSurface)),
    `${theme}: primary action must stay distinct from the success tone (${primarySurface} vs ${successSurface})`,
  ).toBeGreaterThanOrEqual(10);
  await expectContrast(save, "text", 4.5, `${theme}: default button label`);
  await save.hover();
  await expectContrast(save, "text", 4.5, `${theme}: default button label (hover)`);
  await page.mouse.move(0, 0);

  const invalid = page.locator("#runtime-embedding-model");
  await invalid.evaluate((el) => el.setAttribute("aria-invalid", "true"));
  await focusControl(invalid);
  await expectContrast(invalid, "border", 3, `${theme}: invalid input boundary`);
  await expectContrast(invalid, "ring", 3, `${theme}: invalid input ring`);
  await invalid.evaluate((el) => el.removeAttribute("aria-invalid"));
  // Restore the draft so leaving the section never raises the unsaved-changes blocker.
  await invalid.fill("embed-1");

  // Settings appearance: select boundary.
  await page.goto("/settings/appearance");
  await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
  await expectContrast(page.locator("#settings-theme-select"), "border", 3, `${theme}: select boundary`);

  // Group workspace: file checkbox boundary, then the destructive button and its focus.
  await page.goto("/groups/stock%2Falpha");
  await expect(page.getByTestId("group-shell")).toBeVisible();
  await openFolder(page, "Reports");
  await expect(page.locator('[data-testid="library-table"] [data-tone="success"]').first()).toBeVisible();
  await expect(page.locator('[data-testid="library-table"] [data-tone="failure"]').first()).toBeVisible();
  const fileRow = page.getByTestId("library-entry-row").first();
  await expect(fileRow).toBeVisible();
  await expectContrast(
    fileRow.locator('[data-slot="checkbox"]'),
    "border",
    3,
    `${theme}: checkbox boundary`,
  );

  await page.goto("/groups/stock%2Falpha/settings");
  const destructive = page.getByRole("button", { name: "Delete", exact: true }).first();
  await expect(destructive).toBeVisible();
  await expectContrast(destructive, "text", 4.5, `${theme}: destructive button label`);
  await destructive.hover();
  await expectContrast(destructive, "text", 4.5, `${theme}: destructive button label (hover)`);
  await page.mouse.move(0, 0);
  const destructiveRing = await measureRingByKeyboard(page, destructive);
  expect(
    destructiveRing.ratio,
    `${theme}: destructive focus ring (${JSON.stringify(destructiveRing)})`,
  ).toBeGreaterThanOrEqual(3);

  // Secondary button: no product callsite, so this is the one test-only control.
  await page.evaluate(() => {
    const panel = document.createElement("div");
    panel.setAttribute("data-testid", "secondary-button-harness");
    panel.style.position = "fixed";
    panel.style.right = "0";
    panel.style.bottom = "0";
    panel.style.zIndex = "50";
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Secondary";
    button.className = "h-8 rounded-lg bg-secondary px-2.5 text-sm font-medium text-secondary-foreground hover:bg-[color-mix(in_oklch,var(--secondary),var(--foreground)_5%)]";
    panel.appendChild(button);
    document.body.appendChild(panel);
  });
  const secondary = page.getByTestId("secondary-button-harness").locator("button");
  await expectContrast(secondary, "text", 4.5, `${theme}: secondary button label`);
  await secondary.hover();
  await expectContrast(secondary, "text", 4.5, `${theme}: secondary button label (hover)`);

  expect(transport.unmocked).toEqual([]);
}

test.describe("react theme contrast", () => {
  test("light theme keeps readable text and essential control boundaries", async ({ page }) => {
    await runContrastSuite(page, "light");
  });

  test("dark theme keeps readable text and essential control boundaries", async ({ page }) => {
    await runContrastSuite(page, "dark");
  });
});

const METADATA_MATRIX: MetadataIndexFixture[] = [
  { ...WORKSPACE_METADATA, index_id: "idx-ready", status: "ready" },
  { ...WORKSPACE_METADATA, index_id: "idx-building", path: "$.title", status: "building" },
  { ...WORKSPACE_METADATA, index_id: "idx-failed", path: "$.date", status: "failed", error_message: "index build failed" },
  { ...WORKSPACE_METADATA, index_id: "idx-deleting", path: "$.author", status: "deleting" },
];

const VISUAL_MATRIX: { name: string; theme: Theme; locale: string; width: number; height: number }[] = [
  { name: "en-light-desktop", theme: "light", locale: "en", width: 1280, height: 900 },
  { name: "en-dark-desktop", theme: "dark", locale: "en", width: 1280, height: 900 },
  { name: "zh-light-narrow", theme: "light", locale: "zh-CN", width: 390, height: 844 },
  { name: "zh-dark-narrow", theme: "dark", locale: "zh-CN", width: 390, height: 844 },
];

test.describe("react theme rendered matrix", () => {
  for (const profile of VISUAL_MATRIX) {
    test(`captures ${profile.name} across the themed routes`, async ({ page }) => {
      await page.setViewportSize({ width: profile.width, height: profile.height });
      await page.addInitScript((value) => {
        window.localStorage.setItem("context69.theme", value.theme);
        window.localStorage.setItem("context69.locale", value.locale);
      }, { theme: profile.theme, locale: profile.locale });
      const transport = await installAppTransport(page, {
        user: ADMIN_USER,
        settings: true,
        workspace: { metadata: METADATA_MATRIX },
        queue: { processing: STATUS_TASKS, items: { "tone-queued": QUEUE_ITEMS } },
      });

      const dir = `test-results/theme-matrix/${profile.name}`;

      await page.goto("/processing-queue");
      await expect(page.locator("html")).toHaveAttribute("data-theme", profile.theme);
      await expect(page.getByTestId("queue-task-row")).toHaveCount(STATUS_TASKS.length);
      await page.locator('[data-testid="queue-task-row"] button[aria-expanded]').first().click();
      await expect(page.locator('[role="progressbar"]').first()).toBeVisible();
      await page.screenshot({ path: `${dir}-queue.png`, fullPage: true });

      await page.goto("/groups/stock%2Falpha");
      await expect(page.getByTestId("group-shell")).toBeVisible();
      await openFolder(page, "Reports");
      const libraryTable = page.locator('[data-testid="library-table"]');
      await expect(libraryTable.locator('[data-tone="success"]').first()).toBeVisible();
      await expect(libraryTable.locator('[data-tone="failure"]').first()).toBeVisible();
      await page.screenshot({ path: `${dir}-library.png`, fullPage: true });

      await page.goto("/groups/stock%2Falpha/settings");
      await page.getByTestId("metadata-source-key").fill("gov_documents");
      await page.getByTestId("metadata-source-key").press("Enter");
      await expect(page.getByTestId("metadata-row").first()).toBeVisible();
      await page.screenshot({ path: `${dir}-metadata.png`, fullPage: true });

      await page.goto("/settings/appearance");
      await expect(page.getByTestId("settings-theme-select")).toBeVisible();
      await page.screenshot({ path: `${dir}-appearance.png`, fullPage: true });

      await page.goto("/settings/docling");
      await expect(page.locator("#docling-base-url")).toBeVisible();
      await page.screenshot({ path: `${dir}-docling.png`, fullPage: true });

      expect(transport.unmocked).toEqual([]);
    });
  }
});
