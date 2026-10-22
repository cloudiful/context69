import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "@/test-utils/storage";
import { applyTimeZoneSelection } from "@/lib/preferences-store";

import {
  FakeEventSource,
  PREVIEW_DOCUMENT,
  installDomPolyfills,
  installFakeEventSource,
  installSearchBackend,
  openSearch,
  resetAuth,
  resetPreferences,
  resetToasts,
  searchHit,
  searchResponse,
} from "./test-support";

async function firstStream(): Promise<FakeEventSource> {
  await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
  return FakeEventSource.instances[0];
}

describe("react search results and preview", () => {
  beforeAll(() => installDomPolyfills());

  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    applyTimeZoneSelection("UTC");
    resetAuth();
    installFakeEventSource();
  });

  afterEach(() => {
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    FakeEventSource.instances = [];
  });

  it("previews the stored document, keeps a single selection, and opens the hit", async () => {
    const backend = installSearchBackend({ document: PREVIEW_DOCUMENT });
    const router = await openSearch("/search?q=policy");
    const source = await firstStream();

    act(() => {
      source.emit("local", searchResponse("policy", [
        searchHit({ chunk_id: "c1", title: "Policy Brief", score: 0.8 }),
        searchHit({ chunk_id: "c2", title: "Policy Annex", score: 0.7, document_id: 2 }),
      ]));
      source.emit("done", {});
    });

    const items = await screen.findAllByTestId("search-result-item");
    expect(items).toHaveLength(2);

    await userEvent.click(within(items[0]).getByTestId("search-result-preview"));

    const dialog = await screen.findByRole("dialog");
    await waitFor(() => {
      expect(backend.callsFor("GET", "/v1/documents/{document_id}")).toHaveLength(1);
    });
    expect((backend.callsFor("GET", "/v1/documents/{document_id}")[0].params as {
      path: { document_id: number };
    }).path.document_id).toBe(1);

    expect(within(dialog).getByTestId("search-selection-preview").textContent)
      .toContain("Stored document body about policy.");
    expect(within(dialog).getByTestId("search-preview-score")).toBeInTheDocument();

    await userEvent.click(within(dialog).getAllByRole("button", { name: "Close" })[0]);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    await userEvent.click(within(items[1]).getByTestId("search-result-select"));
    const secondDialog = await screen.findByRole("dialog");
    await waitFor(() => {
      expect(backend.callsFor("GET", "/v1/documents/{document_id}")).toHaveLength(2);
    });
    expect((backend.callsFor("GET", "/v1/documents/{document_id}")[1].params as {
      path: { document_id: number };
    }).path.document_id).toBe(2);

    expect(within(items[0]).getByTestId("search-result-select")).toHaveAttribute("aria-pressed", "false");
    expect(within(items[1]).getByTestId("search-result-select")).toHaveAttribute("aria-pressed", "true");

    await userEvent.click(within(secondDialog).getByTestId("search-preview-open"));

    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/documents/2");
    });
    expect(router.state.location.search).toBe("?from=search");

    const session = window.sessionStorage.getItem("context69.search-session") ?? "";
    expect(session).toContain("policy");
    expect(session).not.toContain("chunk_id");
  });

  it("closes the preview when a reranked page replaces the selection", async () => {
    installSearchBackend();
    await openSearch("/search?q=policy");
    const source = await firstStream();

    act(() => {
      source.emit("local", searchResponse("policy", [
        searchHit({ chunk_id: "c1", title: "Policy Brief", score: 0.8 }),
      ]));
    });

    const item = await screen.findByTestId("search-result-preview");
    await userEvent.click(item);
    expect(await screen.findByRole("dialog")).toBeInTheDocument();

    act(() => {
      source.emit("reranked", searchResponse("policy", [
        searchHit({ chunk_id: "c1", title: "Policy Brief", score: 0.9 }),
      ]));
    });

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("explains a failed document preview and offers a retry", async () => {
    installSearchBackend({ documentError: { status: 500, message: "preview unavailable" } });
    await openSearch("/search?q=policy");
    const source = await firstStream();

    act(() => {
      source.emit("local", searchResponse("policy", [
        searchHit({ chunk_id: "c1", title: "Policy Brief", score: 0.8 }),
      ]));
    });

    await userEvent.click(await screen.findByTestId("search-result-preview"));
    const dialog = await screen.findByRole("dialog");

    expect(await within(dialog).findByText("preview unavailable")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(within(dialog).getByText("Failed to load the document preview")).toBeInTheDocument();
  });

  it("labels lower-bound totals, shows the score and provenance, and follows the time zone", async () => {
    installSearchBackend();
    await openSearch("/search?q=policy");
    const source = await firstStream();

    act(() => {
      source.emit("local", searchResponse("policy", [
        searchHit({
          chunk_id: "c1",
          title: "Policy Brief",
          score: 0.8123,
          group_path: "stock/sgs-disclosures",
          published_at: "2026-10-01T08:00:00Z",
          match_reason: "semantic+title",
        }),
      ], {
        total: 9,
        total_pages: 2,
        has_more: true,
        total_is_exact: false,
      }));
    });

    expect(await screen.findByText("At least 9 results")).toBeInTheDocument();
    expect(screen.queryByText("Results: 9")).not.toBeInTheDocument();

    const item = await screen.findByTestId("search-result-item");
    expect(within(item).getByTestId("search-result-score")).toHaveTextContent("0.812");
    expect(within(item).getByText("stock/sgs-disclosures")).toBeInTheDocument();
    expect(within(item).getByText("Semantic match")).toBeInTheDocument();
    expect(within(item).getByText("Title match")).toBeInTheDocument();
    expect(within(item).getByText("2026-10-01")).toBeInTheDocument();
    expect(item.querySelectorAll("mark").length).toBeGreaterThan(0);

    act(() => {
      applyTimeZoneSelection("Pacific/Honolulu");
    });

    await waitFor(() => {
      expect(within(item).getByText("2026-09-30")).toBeInTheDocument();
    });
  });

  it("shows the actionable empty state for a completed search", async () => {
    installSearchBackend();
    await openSearch("/search?q=nothing");
    const source = await firstStream();

    act(() => {
      source.emit("local", searchResponse("nothing", []));
      source.emit("done", {});
    });

    expect(await screen.findByText("No Results")).toBeInTheDocument();
    expect(screen.getByText(
      "No indexed documents matched the current query and filters.",
    )).toBeInTheDocument();
    expect(screen.queryByTestId("search-result-item")).not.toBeInTheDocument();
  });
});
