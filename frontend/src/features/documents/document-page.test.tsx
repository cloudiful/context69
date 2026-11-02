import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider, useParams, useSearchParams } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { applyTimeZoneSelection, setLocale } from "@/lib/preferences-store";
import { saveSearchSession } from "@/utils/search";

import { DocumentPage } from "./document-page";
import {
  chunkFixture,
  documentFixture,
  documentFrom,
  documentResultFixture,
  documentResultsPage,
  installDocumentBackend,
} from "./test-support";

function GroupTargetProbe() {
  const params = useParams();
  const [searchParams] = useSearchParams();
  return (
    <div data-testid="group-target">
      <span data-testid="group-target-path">{params.groupPath}</span>
      <span data-testid="group-target-file">{searchParams.get("file") ?? ""}</span>
      <span data-testid="group-target-from">{searchParams.get("from") ?? ""}</span>
    </div>
  );
}

function renderReader(path: string | string[], initialIndex?: number) {
  const router = createMemoryRouter(
    [
      { path: "/documents/:id", element: <DocumentPage /> },
      { path: "/groups/:groupPath", element: <GroupTargetProbe /> },
      { path: "/search", element: <div data-testid="search-screen">search</div> },
      { path: "/", element: <div data-testid="home-screen">home</div> },
    ],
    { initialEntries: Array.isArray(path) ? path : [path], initialIndex },
  );
  render(
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>,
  );
  return router;
}

async function reader(path = "/documents/42") {
  const router = renderReader(path);
  await screen.findByTestId("document-page");
  return router;
}

describe("document reader", () => {
  beforeEach(() => {
    applyTimeZoneSelection("UTC");
    window.sessionStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    window.sessionStorage.clear();
  });

  it("rejects a non-numeric id without a request", async () => {
    const backend = installDocumentBackend();
    await reader("/documents/12abc");

    expect(await screen.findByTestId("document-invalid-id")).toHaveTextContent("Invalid document id");
    expect(backend.calls).toHaveLength(0);
  });

  it("renders loading, then the title, summary, locales and provenance", async () => {
    const backend = installDocumentBackend({
      deferred: true,
      document: {
        ...documentFixture({
          content_locale: "zh-CN",
          requested_locale: "en-US",
          translation_status: "succeeded" as const,
          is_fallback: false,
        }),
        ...documentFrom('{"metadata_json":{"region":"EU"}}'),
      },
    });
    await reader();

    expect(backend.calls).toHaveLength(1);
    expect(backend.calls[0].signal).toBeInstanceOf(AbortSignal);
    expect(screen.getByTestId("document-loading")).toBeInTheDocument();

    backend.release();

    expect(await screen.findByRole("heading", { name: "Cyber Policy Update" })).toBeInTheDocument();
    expect(screen.getByTestId("document-summary")).toHaveTextContent("Stored summary about cyber policy.");
    expect(screen.getByTestId("document-content-locale")).toHaveTextContent("Content zh-CN");
    expect(screen.getByTestId("document-requested-locale")).toHaveTextContent("Requested en-US");
    expect(screen.getByTestId("document-translation-status")).toHaveTextContent("Translated");
    expect(screen.getByTestId("document-block")).toHaveTextContent("First stored block.");

    const provenance = screen.getByTestId("document-provenance");
    expect(provenance).toHaveTextContent("hash-42");
    expect(provenance).toHaveTextContent("stock/alpha");
    expect(provenance).toHaveTextContent("ext-42");
    expect(provenance).toHaveTextContent("2026-01-02 08:30");
    expect(provenance).toHaveTextContent('"region": "EU"');
  });

  it("localizes the header badge and provenance visibility in both locales", async () => {
    installDocumentBackend({ document: documentFixture({ visibility: "private" }) });
    await reader();

    const badge = await screen.findByTestId("document-visibility");
    expect(badge).toHaveTextContent("Private");
    expect(badge).not.toHaveTextContent("private");
    const provenance = screen.getByTestId("document-provenance");
    expect(within(provenance).getByText("Private")).toBeInTheDocument();
    expect(within(provenance).queryByText("private")).not.toBeInTheDocument();

    setLocale("zh-CN");
    await waitFor(() => {
      expect(screen.getByTestId("document-visibility")).toHaveTextContent("私有");
      expect(within(screen.getByTestId("document-provenance")).getByText("私有")).toBeInTheDocument();
    });
    expect(screen.getByTestId("document-visibility")).not.toHaveTextContent("private");

    setLocale("en");
    await waitFor(() => expect(screen.getByTestId("document-visibility")).toHaveTextContent("Private"));
  });

  it("keeps an unknown future visibility visible instead of hiding it", async () => {
    installDocumentBackend({ document: documentFixture({ visibility: "future_visibility" as never }) });
    await reader();

    expect(await screen.findByTestId("document-visibility")).toHaveTextContent("future_visibility");
    expect(within(screen.getByTestId("document-provenance")).getByText("future_visibility")).toBeInTheDocument();
  });

  it("shows a not-found state without a retry", async () => {
    installDocumentBackend({ failure: { status: 404, message: "missing" } });
    await reader();

    const notFound = await screen.findByTestId("document-error");
    expect(notFound).toHaveTextContent("Document not found");
    expect(notFound).toHaveTextContent("does not exist");
    expect(within(notFound).queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
  });

  it("shows a forbidden state without a retry", async () => {
    installDocumentBackend({ failure: { status: 403, message: "denied" } });
    await reader();

    const forbidden = await screen.findByTestId("document-error");
    expect(forbidden).toHaveTextContent("Document unavailable");
    expect(forbidden).toHaveTextContent("do not have access");
    expect(within(forbidden).queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
  });

  it("recovers from a network failure through the explicit retry", async () => {
    const backend = installDocumentBackend({ failure: { status: 500, message: "boom" } });
    await reader();

    const alert = await screen.findByTestId("document-error");
    expect(alert).toHaveTextContent("Document load failed");

    backend.setFailure(null);
    await userEvent.setup().click(within(alert).getByRole("button", { name: "Retry" }));

    expect(await screen.findByRole("heading", { name: "Cyber Policy Update" })).toBeInTheDocument();
    expect(backend.calls.length).toBeGreaterThanOrEqual(2);
  });

  it("aborts the previous read and drops its content when the route id changes", async () => {
    const backend = installDocumentBackend({
      document: (documentId) => documentFixture({
        document_id: documentId,
        title: `Document ${documentId}`,
        chunks: [chunkFixture({ text: `body-${documentId}` })],
      }),
    });
    const router = await reader("/documents/42");

    expect(await screen.findByRole("heading", { name: "Document 42" })).toBeInTheDocument();
    const firstSignal = backend.calls[0].signal as AbortSignal;

    await router.navigate("/documents/43");

    expect(await screen.findByRole("heading", { name: "Document 43" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Document 42" })).not.toBeInTheDocument();
    expect(screen.queryByText("body-42")).not.toBeInTheDocument();
    expect(backend.calls.map((call) => call.documentId)).toEqual([42, 43]);
    await waitFor(() => expect(firstSignal.aborted).toBe(true));
  });

  it("re-renders the published date for the active display zone", async () => {
    installDocumentBackend({
      document: documentFixture({ published_at: "2026-01-01T20:00:00Z" }),
    });
    await reader();

    expect(await screen.findByText("2026-01-01")).toBeInTheDocument();

    applyTimeZoneSelection("Asia/Tokyo");

    expect(await screen.findByText("2026-01-02")).toBeInTheDocument();
  });

  it("pages stored blocks ten at a time and resets for a new document", async () => {
    const backend = installDocumentBackend({
      document: (documentId) => documentFixture({
        document_id: documentId,
        title: `Document ${documentId}`,
        chunks: Array.from({ length: 12 }, (_, index) => chunkFixture({
          chunk_id: `chunk-${index}`,
          chunk_index: index,
          text: `block-${index + 1}`,
        })),
      }),
    });
    const user = userEvent.setup();
    const router = await reader();

    await waitFor(() => expect(screen.getAllByTestId("document-block")).toHaveLength(10));
    expect(screen.getByTestId("document-page-status")).toHaveTextContent("Page 1 of 2");
    expect(screen.queryByText("block-11")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Next" }));

    expect(screen.getAllByTestId("document-block")).toHaveLength(2);
    expect(screen.getByText("block-11")).toBeInTheDocument();
    expect(screen.getByTestId("document-page-status")).toHaveTextContent("Page 2 of 2");

    await router.navigate("/documents/43");
    await screen.findByRole("heading", { name: "Document 43" });
    expect(screen.getByTestId("document-page-status")).toHaveTextContent("Page 1 of 2");
    expect(backend.calls.map((call) => call.documentId)).toEqual([42, 43]);
  });

  it("rejects empty and stale blocks while keeping the valid ones", async () => {
    installDocumentBackend({
      document: documentFixture({
        chunks: [
          chunkFixture({ chunk_id: "ok", chunk_index: 0, text: "current text" }),
          chunkFixture({ chunk_id: "stale", chunk_index: 1, record_hash: "older", text: "stale secret text" }),
          chunkFixture({ chunk_id: "empty", chunk_index: 2, text: "  " }),
        ],
      }),
    });
    await reader();

    expect(await screen.findByText("current text")).toBeInTheDocument();
    expect(screen.queryByText("stale secret text")).not.toBeInTheDocument();
    expect(screen.getAllByTestId("document-invalid-block")).toHaveLength(2);
    expect(screen.getAllByTestId("document-invalid-block")[0]).toHaveTextContent(
      "This block changed while the document was loading",
    );
    expect(screen.getAllByTestId("document-invalid-block")[1]).toHaveTextContent(
      "This block is empty",
    );
  });

  it("shows an empty-content state without inventing blocks", async () => {
    installDocumentBackend({ document: documentFixture({ chunks: [], summary: null }) });
    await reader();

    expect(await screen.findByTestId("document-no-blocks")).toHaveTextContent("No Content");
    expect(screen.queryByTestId("document-block")).not.toBeInTheDocument();
  });

  it("marks the current search terms only when returning from search", async () => {
    saveSearchSession({
      query: "cyber policy",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
      groupPath: "",
      sort: "relevance",
    }, 2);
    installDocumentBackend();
    await reader("/documents/42?from=search");

    const summary = await screen.findByTestId("document-summary");
    expect(within(summary).getAllByText(/cyber/i).length).toBeGreaterThan(0);
    expect(summary.querySelectorAll("mark").length).toBeGreaterThan(0);
    expect(screen.getByTestId("document-back-to-search")).toBeInTheDocument();
    expect(screen.getByTestId("document-block-content").querySelectorAll("mark").length).toBe(0);
  });

  it("returns through browser history before using the stored session", async () => {
    saveSearchSession({
      query: "ignored-session",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 8,
      groupPath: "",
      sort: "relevance",
    }, 1);
    const originalHistoryLength = Object.getOwnPropertyDescriptor(window.history, "length");
    Object.defineProperty(window.history, "length", { configurable: true, get: () => 3 });
    try {
      installDocumentBackend();
      const router = renderReader(["/search", "/documents/42?from=search"], 1);
      await screen.findByRole("heading", { name: "Cyber Policy Update" });
      await userEvent.setup().click(screen.getByTestId("document-back-to-search"));

      await waitFor(() => expect(router.state.location.pathname).toBe("/search"));
      expect(router.state.location.search).toBe("");
      expect(screen.getByTestId("search-screen")).toBeInTheDocument();
    } finally {
      if (originalHistoryLength) {
        Object.defineProperty(window.history, "length", originalHistoryLength);
      } else {
        delete (window.history as unknown as { length?: number }).length;
      }
    }
  });

  it("falls back to the stored session when there is no browser history", async () => {
    saveSearchSession({
      query: "cyber",
      sourceKey: "",
      publishedAfter: "",
      publishedBefore: "",
      limit: 16,
      groupPath: "",
      sort: "date",
    }, 3);
    installDocumentBackend();
    const router = await reader("/documents/42?from=search");
    await screen.findByRole("heading", { name: "Cyber Policy Update" });
    await userEvent.setup().click(screen.getByTestId("document-back-to-search"));

    await waitFor(() => expect(router.state.location.pathname).toBe("/search"));
    expect(router.state.location.search).toContain("q=cyber");
    expect(router.state.location.search).toContain("limit=16");
    expect(router.state.location.search).toContain("sort=date");
  });

  it("falls back to the search page when no session exists", async () => {
    installDocumentBackend();
    const router = await reader("/documents/42?from=search");
    await screen.findByRole("heading", { name: "Cyber Policy Update" });
    await userEvent.setup().click(screen.getByTestId("document-back-to-search"));

    await waitFor(() => expect(router.state.location.pathname).toBe("/search"));
    expect(router.state.location.search).toBe("");
    expect(screen.getByTestId("search-screen")).toBeInTheDocument();
  });

  it("opens the library file with the encoded group path and the return marker", async () => {
    installDocumentBackend({
      document: documentFixture({
        is_library_file: true,
        library_file_id: "file-42",
        group_path: "stock/alpha",
      }),
    });
    const router = await reader("/documents/42?from=search");

    const open = await screen.findByTestId("document-open-source");
    expect(open).toHaveTextContent("Open In Library");
    await userEvent.setup().click(open);

    await waitFor(() => expect(router.state.location.pathname).toBe("/groups/stock%2Falpha"));
    expect(await screen.findByTestId("group-target-path")).toHaveTextContent("stock/alpha");
    expect(screen.getByTestId("group-target-file")).toHaveTextContent("file-42");
    expect(screen.getByTestId("group-target-from")).toHaveTextContent("search");
  });

  it("opens a safe external origin with noopener and noreferrer", async () => {
    const openSpy = vi.fn();
    vi.stubGlobal("open", openSpy);
    installDocumentBackend({ document: documentFixture({ source_uri: "https://example.test/doc" }) });
    await reader();

    await userEvent.setup().click(await screen.findByTestId("document-open-source"));

    expect(openSpy).toHaveBeenCalledWith("https://example.test/doc", "_blank", "noopener,noreferrer");
  });

  it("refuses an unsafe origin link", async () => {
    const openSpy = vi.fn();
    vi.stubGlobal("open", openSpy);
    installDocumentBackend({ document: documentFixture({ source_uri: "javascript:alert(1)" }) });
    await reader();

    const open = await screen.findByTestId("document-open-source");
    expect(open).toBeDisabled();

    await userEvent.setup().click(open).catch(() => undefined);
    expect(openSpy).not.toHaveBeenCalled();
  });

  it("never executes or links hostile markdown", async () => {
    vi.stubGlobal("fetch", vi.fn());
    const hostile = [
      "<script>window.__p3cXss = 1</script>",
      "<img src=x onerror=\"window.__p3cXss = 2\">",
      "<iframe src=\"javascript:window.__p3cXss = 3\"></iframe>",
      "[click me](javascript:window.__p3cXss = 4)",
      "[safe](https://example.test/ok)",
      "[data](data:text/html;base64,PHNjcmlwdD53aW5kb3cuX19wM2NYc3M9NTwvc2NyaXB0Pg==)",
    ].join("\n\n");
    installDocumentBackend({
      document: documentFixture({
        summary: null,
        chunks: [chunkFixture({ text: hostile })],
      }),
    });

    const router = await reader();
    await screen.findByTestId("document-block-content");

    expect((window as unknown as { __p3cXss?: number }).__p3cXss).toBeUndefined();
    const content = screen.getByTestId("document-block-content");
    expect(content.querySelector("script")).toBeNull();
    expect(content.querySelector("img")).toBeNull();
    expect(content.querySelector("iframe")).toBeNull();
    expect(content.querySelectorAll('a[href^="javascript:"]').length).toBe(0);
    expect(content.querySelectorAll('a[href^="data:"]').length).toBe(0);
    expect(within(content).getByText("click me")).toBeInTheDocument();
    expect(within(content).getByText("data")).toBeInTheDocument();

    const safeLink = within(content).getByRole("link", { name: "safe" });
    expect(safeLink).toHaveAttribute("href", "https://example.test/ok");
    expect(safeLink).toHaveAttribute("rel", "noreferrer noopener");
    expect(safeLink).toHaveAttribute("target", "_blank");

    // Navigating away before the hostile text could ever be interpreted leaves
    // the app usable and the injected global still unset.
    await router.navigate("/documents/43");
    expect((window as unknown as { __p3cXss?: number }).__p3cXss).toBeUndefined();
  });

  it("shows current latest results grouped by producer and kind", async () => {
    installDocumentBackend({
      results: documentResultsPage([
        documentResultFixture({
          id: 2,
          result_kind: "sentiment.v1",
          result: { direction: "positive", note: "<img src=x onerror=alert(1)>" },
        }),
        documentResultFixture({
          id: 1,
          producer: "stock.disclosure",
          result_kind: "summary.v1",
          result: { headline: "Filed" },
        }),
      ]),
    });
    await reader();

    expect(await screen.findByTestId("document-results")).toHaveTextContent("Results");
    const groups = await screen.findAllByTestId("document-result-group");
    expect(groups).toHaveLength(2);
    expect(groups[0]).toHaveTextContent("stock.disclosure");
    expect(groups[1]).toHaveTextContent("stock.news");
    expect(within(groups[1]).getByTestId("document-result-kind")).toHaveTextContent("sentiment.v1");
    expect(within(groups[1]).getAllByTestId("document-result-field")[0]).toHaveTextContent("direction");
    expect(groups[1].querySelector("img")).toBeNull();
    expect(screen.getByTestId("document-results-current")).toHaveAttribute("aria-pressed", "true");
  });

  it("keeps current producer results reachable beyond the first page", async () => {
    installDocumentBackend({
      results: (_, query) => {
        if (query?.after_id === 2) {
          return documentResultsPage([
            documentResultFixture({ id: 1, producer: "last-producer", result: { marker: "last current result" } }),
          ]);
        }
        return documentResultsPage(
          Array.from({ length: 50 }, (_, index) => documentResultFixture({
            id: 51 - index,
            producer: `producer-${index}`,
          })),
          { next_after_id: 2 },
        );
      },
    });
    const user = userEvent.setup();
    await reader();
    await screen.findAllByTestId("document-result-group");
    const loadMore = screen.queryByRole("button", { name: /load more/i });
    if (loadMore) await user.click(loadMore);
    expect(await screen.findByText("last-producer")).toBeInTheDocument();
  });

  it("does not mix old source results into Current when the source changes between pages", async () => {
    let moved = false;
    installDocumentBackend({
      results: (_, query) => {
        if (query?.after_id === 2) {
          moved = true;
          return documentResultsPage([], { source_record_hash: "new-hash" });
        }
        if (moved) {
          return documentResultsPage([
            documentResultFixture({ id: 60, producer: "new-source-producer", source_record_hash: "new-hash" }),
          ], { source_record_hash: "new-hash" });
        }
        return documentResultsPage(Array.from({ length: 50 }, (_, index) =>
          documentResultFixture({
            id: 51 - index,
            producer: index === 0 ? "old-source-producer" : `old-producer-${index}`,
            source_record_hash: "hash-42",
          })), { next_after_id: 2, source_record_hash: "hash-42" });
      },
    });
    const user = userEvent.setup();
    await reader();
    await screen.findByText("old-source-producer");
    await user.click(screen.getByRole("button", { name: /load more/i }));
    await waitFor(() => expect(screen.queryByText("old-source-producer")).not.toBeInTheDocument());
    expect(await screen.findByText("new-source-producer")).toBeInTheDocument();
  });

  it("finishes a Current reset to an empty source version without a refetch loop", async () => {
    let moved = false;
    const cursors: (number | undefined)[] = [];
    installDocumentBackend({
      results: (_, query) => {
        cursors.push(query?.after_id);
        if (query?.after_id === 2) moved = true;
        if (moved) return documentResultsPage([], { source_record_hash: "empty-new-hash" });
        return documentResultsPage(Array.from({ length: 50 }, (_, index) =>
          documentResultFixture({ id: 51 - index, producer: `empty-reset-producer-${index}` })),
        { next_after_id: 2 });
      },
    });
    const user = userEvent.setup();
    await reader();
    await screen.findByText("empty-reset-producer-0");
    await user.click(screen.getByRole("button", { name: /load more/i }));
    expect(await screen.findByTestId("document-results-empty")).toHaveTextContent(
      "No results for the current source version.",
    );
    expect(screen.queryByText("empty-reset-producer-0")).not.toBeInTheDocument();
    expect(cursors).toEqual([undefined, 2, undefined]);
  });

  it("lists stored history with current and historical source-version labels", async () => {
    installDocumentBackend({
      results: (_, query) => {
        if (query?.current_only) {
          return documentResultsPage([
            documentResultFixture({ id: 3, idempotency_key: "current", source_record_hash: "hash-42" }),
          ]);
        }
        if (query?.after_id === 2) {
          return documentResultsPage([
            documentResultFixture({ id: 1, idempotency_key: "old", source_record_hash: "hash-old" }),
          ]);
        }
        return documentResultsPage(
          [
            documentResultFixture({ id: 3, idempotency_key: "current", source_record_hash: "hash-42" }),
            documentResultFixture({ id: 2, idempotency_key: "older", source_record_hash: "hash-old" }),
          ],
          { next_after_id: 2 },
        );
      },
    });
    const user = userEvent.setup();
    await reader();
    await screen.findByTestId("document-results");

    await user.click(screen.getByTestId("document-results-history"));

    const items = await screen.findAllByTestId("document-result-history-item");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Current");
    expect(items[1]).toHaveTextContent("Historical");
    expect(items[1]).toHaveTextContent("Source version: hash-old");
    expect(within(items[0]).getByText("direction")).toBeInTheDocument();
    expect(within(items[0]).getByText("positive")).toBeInTheDocument();

    await user.click(screen.getByTestId("document-results-load-more"));
    await waitFor(() => expect(screen.getAllByTestId("document-result-history-item")).toHaveLength(3));
    expect(screen.queryByTestId("document-results-load-more")).not.toBeInTheDocument();
  });

  it("shows a retryable results error without breaking the document reader", async () => {
    const backend = installDocumentBackend({
      resultsFailure: { status: 500, message: "results unavailable" },
    });
    const user = userEvent.setup();
    await reader();

    expect(await screen.findByTestId("document-results-error")).toHaveTextContent(
      "Results could not be loaded.",
    );
    expect(screen.getByTestId("document-summary")).toBeInTheDocument();

    backend.setResultsFailure(null);
    await user.click(within(screen.getByTestId("document-results-error")).getByRole("button", {
      name: "Retry",
    }));

    expect(await screen.findByTestId("document-results-empty")).toHaveTextContent(
      "No results for the current source version.",
    );
  });

  it("renders scalar result fields as labelled key and value text", async () => {
    installDocumentBackend({
      results: documentResultsPage([
        documentResultFixture({
          result: { headline: "Filed", score: 0, confident: false, note: null },
        }),
      ]),
    });
    await reader();

    const fields = await screen.findAllByTestId("document-result-field");
    expect(fields).toHaveLength(4);

    const field = (name: string) => fields.find((row) => within(row).queryByText(name));
    expect(within(field("headline") as HTMLElement).getByText("Filed")).toBeInTheDocument();
    expect(within(field("score") as HTMLElement).getByText("0")).toBeInTheDocument();
    expect(within(field("confident") as HTMLElement).getByText("false")).toBeInTheDocument();
    expect(within(field("note") as HTMLElement).getByText("null")).toBeInTheDocument();
  });

  it("shows a meaningful empty state for an empty result object", async () => {
    installDocumentBackend({
      results: documentResultsPage([documentResultFixture({ result: {} })]),
    });
    await reader();

    expect(await screen.findByTestId("document-result-empty-payload")).toHaveTextContent(
      "This result has no fields.",
    );
    expect(screen.queryByTestId("document-result-field")).not.toBeInTheDocument();
  });

  it("keeps hostile and Unicode result text inert while rendering it readably", async () => {
    const hostile = "<img src=x onerror=window.__resultFieldXss=1><script>window.__resultFieldXss=2</script>";
    installDocumentBackend({
      results: documentResultsPage([
        documentResultFixture({ result: { note: hostile, unicode: "日本語 ✓" } }),
      ]),
    });
    await reader();

    const fields = await screen.findAllByTestId("document-result-field");
    const container = fields[0].parentElement as HTMLElement;
    expect(within(container).getByText(hostile)).toBeInTheDocument();
    expect(within(container).getByText("日本語 ✓")).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("script")).toBeNull();
    expect((window as unknown as { __resultFieldXss?: number }).__resultFieldXss).toBeUndefined();

    await userEvent.setup().click(screen.getAllByText("Original JSON")[0]);
    const payload = screen.getByTestId("document-result-payload");
    expect(payload).toHaveTextContent(hostile);
    expect(payload.querySelector("img")).toBeNull();
    expect(payload.querySelector("script")).toBeNull();
  });

  it("summarizes nested objects and arrays with expandable raw JSON", async () => {
    installDocumentBackend({
      results: documentResultsPage([
        documentResultFixture({
          result: { meta: { region: "EU", tags: ["alpha", "beta"] }, rows: [1, 2, 3] },
        }),
      ]),
    });
    const user = userEvent.setup();
    await reader();

    const nested = await screen.findAllByTestId("document-result-nested");
    expect(nested).toHaveLength(2);
    expect(nested[0]).toHaveTextContent("Object · 2 fields");
    expect(nested[1]).toHaveTextContent("Array · 3 items");
    expect(nested[0]).not.toHaveAttribute("open");
    expect(nested[0].querySelector("img")).toBeNull();

    await user.click(within(nested[0]).getByText("Object · 2 fields"));

    expect(nested[0]).toHaveAttribute("open");
    expect(nested[0].querySelector("pre")).toHaveTextContent('"region": "EU"');
  });

  it("bounds a long string visually and expands it accessibly", async () => {
    const longValue = `${"segment ".repeat(40)}TAIL-END`;
    installDocumentBackend({
      results: documentResultsPage([documentResultFixture({ result: { body: longValue } })]),
    });
    const user = userEvent.setup();
    await reader();

    const value = await screen.findByTestId("document-result-string");
    const toggle = screen.getByTestId("document-result-string-toggle");
    expect(toggle).toHaveTextContent("Show full value");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(value.className).toContain("line-clamp-3");

    await user.click(toggle);

    expect(toggle).toHaveTextContent("Show less");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(value.className).not.toContain("line-clamp-3");
    expect(value).toHaveTextContent("TAIL-END");
  });

  it("limits wide results and lets every field be shown", async () => {
    const result = Object.fromEntries(
      Array.from({ length: 14 }, (_, index) => [`field-${index}`, `value-${index}`]),
    );
    installDocumentBackend({
      results: documentResultsPage([documentResultFixture({ result })]),
    });
    const user = userEvent.setup();
    await reader();

    expect(await screen.findAllByTestId("document-result-field")).toHaveLength(10);
    const showMore = screen.getByTestId("document-result-show-more");
    expect(showMore).toHaveTextContent("Show 4 more fields");

    await user.click(showMore);

    expect(screen.getAllByTestId("document-result-field")).toHaveLength(14);
    expect(screen.getByText("value-13")).toBeInTheDocument();

    await user.click(screen.getByTestId("document-result-show-less"));
    expect(screen.getAllByTestId("document-result-field")).toHaveLength(10);
  });

  it("keeps complete original JSON and copies the full payload", async () => {
    const result = {
      headline: "Filed",
      meta: { region: "EU", tags: ["alpha"] },
    };
    const writeText = vi.fn().mockResolvedValue(undefined);
    installDocumentBackend({
      results: documentResultsPage([documentResultFixture({ result })]),
    });
    const user = userEvent.setup({ writeToClipboard: false });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    await reader();

    await user.click((await screen.findAllByText("Original JSON"))[0]);

    const payload = screen.getByTestId("document-result-payload");
    expect(payload.textContent).toBe(JSON.stringify(result, null, 2));
    expect(payload).toHaveTextContent('"region": "EU"');

    await user.click(screen.getByTestId("document-result-copy"));

    await waitFor(() => {
      expect(screen.getByTestId("document-result-copy-status")).toHaveTextContent(
        "Copied to clipboard",
      );
    });
    expect(writeText).toHaveBeenCalledWith(JSON.stringify(result, null, 2));
  });

  it("reports failed and unavailable clipboard writes without claiming success", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    installDocumentBackend({
      results: documentResultsPage([documentResultFixture({ result: { headline: "Filed" } })]),
    });
    const user = userEvent.setup({ writeToClipboard: false });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    await reader();

    await user.click((await screen.findAllByText("Original JSON"))[0]);
    await user.click(screen.getByTestId("document-result-copy"));

    await waitFor(() => {
      expect(screen.getByTestId("document-result-copy-status")).toHaveTextContent("Could not copy");
    });
    expect(screen.getByTestId("document-result-copy-status")).not.toHaveTextContent(
      "Copied to clipboard",
    );

    Object.defineProperty(navigator, "clipboard", { configurable: true, value: undefined });
    await user.click(screen.getByTestId("document-result-copy"));

    await waitFor(() => {
      expect(screen.getByTestId("document-result-copy-status")).toHaveTextContent(
        "Clipboard unavailable",
      );
    });
    expect(writeText).toHaveBeenCalledTimes(1);
  });
});
