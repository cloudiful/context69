import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "@/test-utils/storage";

import {
  installDomPolyfills,
  installFakeBackend,
  openSettings,
  resetAuth,
  resetPreferences,
  resetToasts,
} from "./test-support";

function readTheme(): string | null {
  return window.localStorage.getItem("context69.theme");
}

describe("appearance settings", () => {
  beforeAll(() => installDomPolyfills());

  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    resetAuth();
  });

  afterEach(() => {
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    delete document.documentElement.dataset.theme;
    document.documentElement.classList.remove("dark");
    document.documentElement.lang = "en";
  });

  it("loads the stored appearance into the controls", async () => {
    installFakeBackend();
    await openSettings("/settings/appearance");

    expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "Appearance" })).toBeInTheDocument();
    expect(screen.getByTestId("settings-locale-select")).toHaveTextContent("English");
    expect(screen.getByTestId("settings-theme-select")).toHaveTextContent("System");
    expect(screen.getByTestId("settings-time-zone-select")).toHaveTextContent("Automatic (");
    expect(screen.getByTestId("settings-save")).toBeDisabled();
  });

  it("keeps an appearance edit as a draft until Save", async () => {
    installFakeBackend();
    await openSettings("/settings/appearance");

    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Dark" }));

    expect(await screen.findByText("Unsaved changes")).toBeInTheDocument();
    expect(readTheme()).toBe("system");
    expect(document.documentElement.dataset.theme).not.toBe("dark");
    expect(screen.getByTestId("settings-save")).toBeEnabled();

    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(readTheme()).toBe("dark");
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect((await screen.findAllByText("Settings saved")).length).toBeGreaterThan(0);
    expect(screen.getByTestId("settings-save")).toBeDisabled();
  });

  it("searches the IANA time zone list and saves the selection", async () => {
    installFakeBackend();
    await openSettings("/settings/appearance");

    await userEvent.click(screen.getByTestId("settings-time-zone-select"));
    await userEvent.type(
      await screen.findByPlaceholderText("Search time zones"),
      "Tokyo",
    );
    await userEvent.click(await screen.findByText("Asia/Tokyo"));

    expect(screen.getByTestId("settings-time-zone-select")).toHaveTextContent("Asia/Tokyo");

    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(window.localStorage.getItem("context69.timeZone")).toBe("Asia/Tokyo");
    });
  });

  it("applies a saved locale and persists it", async () => {
    installFakeBackend();
    await openSettings("/settings/appearance");

    await userEvent.click(screen.getByTestId("settings-locale-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Simplified Chinese" }));
    await userEvent.click(screen.getByTestId("settings-save"));

    await waitFor(() => {
      expect(window.localStorage.getItem("context69.locale")).toBe("zh-CN");
    });
    await waitFor(() => {
      expect(document.documentElement.lang).toBe("zh-CN");
    });
  });

  it("warns and stays pending when the browser denies storage", async () => {
    installFakeBackend();
    await openSettings("/settings/appearance");

    const storage = window.localStorage;
    const setItem = vi.spyOn(storage, "setItem").mockImplementation(() => {
      throw new DOMException("Storage denied", "SecurityError");
    });

    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Dark" }));
    await userEvent.click(screen.getByTestId("settings-save"));

    expect((await screen.findAllByText(/could not store the appearance/)).length).toBeGreaterThan(0);
    expect(screen.queryByText("Settings saved")).not.toBeInTheDocument();
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(screen.getByTestId("settings-save")).toBeEnabled();
    setItem.mockRestore();
  });

  it("keeps Save enabled after reverting the draft to the unstored baseline", async () => {
    installFakeBackend();
    await openSettings("/settings/appearance");

    const storage = window.localStorage;
    vi.spyOn(storage, "setItem").mockImplementation(() => {
      throw new DOMException("Storage denied", "SecurityError");
    });

    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Dark" }));
    await userEvent.click(screen.getByTestId("settings-save"));
    expect((await screen.findAllByText(/could not store the appearance/)).length).toBeGreaterThan(0);

    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "System" }));

    expect(screen.getByTestId("settings-theme-select")).toHaveTextContent("System");
    expect(screen.getByTestId("settings-save")).toBeEnabled();
  });

  it("keeps the draft across section changes after confirming the leave", async () => {
    installFakeBackend();
    const router = await openSettings("/settings/appearance");

    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Dark" }));

    await userEvent.click(within(screen.getByRole("navigation", { name: "Settings" })).getByRole("link", { name: "Search" }));

    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("You have unsaved settings changes. Leave anyway?")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Leave" }));

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/settings/search");
    });

    await userEvent.click(within(screen.getByRole("navigation", { name: "Settings" })).getByRole("link", { name: "Appearance" }));
    const secondDialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(secondDialog).getByRole("button", { name: "Leave" }));

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/settings/appearance");
    });
    expect(await screen.findByTestId("settings-theme-select")).toHaveTextContent("Dark");
  });

  it("stays on the page when the leave confirmation is cancelled", async () => {
    installFakeBackend();
    const router = await openSettings("/settings/appearance");

    await userEvent.click(screen.getByTestId("settings-theme-select"));
    await userEvent.click(await screen.findByRole("option", { name: "Dark" }));

    await userEvent.click(within(screen.getByRole("navigation", { name: "Settings" })).getByRole("link", { name: "Search" }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/settings/appearance");
    });
    expect(screen.getByTestId("settings-theme-select")).toHaveTextContent("Dark");
  });
});
