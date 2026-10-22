import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider, useParams, useSearchParams } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { applyTimeZoneSelection, setLocale } from "@/lib/preferences-store";
import { saveSearchSession } from "@/utils/search";

import { DocumentPage } from "./document-page";
import { chunkFixture, documentFixture, documentFrom, installDocumentBackend } from "./test-support";

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
});
