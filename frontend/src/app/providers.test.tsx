import { useQuery } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearSession, restoreSession } from "@/lib/auth-store";
import { apiClient } from "@/services/api";
import type { AuthUserResponse, DocumentResponse } from "@/services/api";

import { AppProviders } from "./providers";

const USER_A: AuthUserResponse = {
  user_id: 1,
  login_name: "alice",
  display_name: "Alice",
  is_admin: false,
  personal_group_path: "personal-alice",
  personal_group_role: "owner",
};

const USER_B: AuthUserResponse = {
  user_id: 2,
  login_name: "bob",
  display_name: "Bob",
  is_admin: false,
  personal_group_path: "personal-bob",
  personal_group_role: "owner",
};

let currentUser: AuthUserResponse | null = null;

function DocumentProbe() {
  const [draft, setDraft] = useState("");
  const query = useQuery({
    queryKey: ["document", 42],
    queryFn: () => apiClient.getDocument(42),
    retry: false,
  });

  return (
    <div>
      <span data-testid="draft">{draft}</span>
      <button type="button" onClick={() => setDraft("typed draft")}>type</button>
      <span data-testid="document">
        {query.data?.title ?? (query.error ? "denied" : "loading")}
      </span>
    </div>
  );
}

async function signIn(user: AuthUserResponse) {
  currentUser = user;
  await act(async () => {
    await restoreSession();
  });
}

async function signOut() {
  currentUser = null;
  await act(async () => {
    clearSession();
  });
}

function renderProbe() {
  return render(
    <AppProviders>
      <DocumentProbe />
    </AppProviders>,
  );
}

describe("AppProviders identity boundary", () => {
  beforeEach(() => {
    currentUser = null;
    clearSession(null);
    vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/v1/auth/me")) {
        return currentUser
          ? new Response(JSON.stringify({ user: currentUser }), {
              status: 200,
              headers: { "Content-Type": "application/json" },
            })
          : new Response(JSON.stringify({ code: "unauthorized", message: "No session" }), {
              status: 401,
              headers: { "Content-Type": "application/json" },
            });
      }
      return new Response(null, { status: 204 });
    }));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    clearSession(null);
  });

  it("never serves one account's cached document to another account", async () => {
    const reads: string[] = [];
    vi.spyOn(apiClient, "getDocument").mockImplementation(async () => {
      const identity = currentUser?.login_name ?? "guest";
      reads.push(identity);
      if (identity === "alice") {
        return { title: "Alice private memo" } as unknown as DocumentResponse;
      }
      throw new Error("denied");
    });

    renderProbe();
    await signIn(USER_A);
    expect(await screen.findByText("Alice private memo")).toBeInTheDocument();

    await signOut();
    await signIn(USER_B);

    await waitFor(() => expect(screen.getByTestId("document")).toHaveTextContent("denied"));
    expect(screen.queryByText("Alice private memo")).not.toBeInTheDocument();
    expect(reads.at(-1)).toBe("bob");
  });

  it("ignores a retired response that arrives after the account switched", async () => {
    let resolveRetired: ((value: DocumentResponse) => void) | null = null;
    const getDocument = vi.spyOn(apiClient, "getDocument").mockImplementation(() => Promise.reject(new Error("denied")));

    renderProbe();
    await waitFor(() => expect(screen.getByTestId("document")).toHaveTextContent("denied"));

    getDocument.mockImplementationOnce(() => new Promise((resolve) => {
      resolveRetired = resolve as (value: DocumentResponse) => void;
    }));
    await signIn(USER_A);
    expect(screen.getByTestId("document")).toHaveTextContent("loading");
    getDocument.mockImplementation(() => Promise.reject(new Error("denied")));

    await signOut();
    await signIn(USER_B);
    await waitFor(() => expect(screen.getByTestId("document")).toHaveTextContent("denied"));

    await act(async () => {
      resolveRetired?.({ title: "Alice private memo" } as unknown as DocumentResponse);
    });

    expect(screen.getByTestId("document")).toHaveTextContent("denied");
    expect(screen.queryByText("Alice private memo")).not.toBeInTheDocument();
  });

  it("starts a fresh cache when the same account signs in again after a guest session", async () => {
    let aliceReads = 0;
    vi.spyOn(apiClient, "getDocument").mockImplementation(async () => {
      if (currentUser?.login_name !== "alice") {
        throw new Error("denied");
      }
      aliceReads += 1;
      return {
        title: aliceReads === 1 ? "Alice first session" : "Alice second session",
      } as unknown as DocumentResponse;
    });

    renderProbe();
    await signIn(USER_A);
    expect(await screen.findByText("Alice first session")).toBeInTheDocument();

    await signOut();
    await signIn(USER_A);

    expect(await screen.findByText("Alice second session")).toBeInTheDocument();
    expect(screen.queryByText("Alice first session")).not.toBeInTheDocument();
    expect(aliceReads).toBe(2);
  });

  it("drops local drafts with the account's subtree", async () => {
    vi.spyOn(apiClient, "getDocument").mockImplementation(async () => ({
      title: "shared document",
    }) as unknown as DocumentResponse);
    const user = userEvent.setup();

    renderProbe();
    await signIn(USER_A);
    await screen.findByText("shared document");

    await user.click(screen.getByRole("button", { name: "type" }));
    expect(screen.getByTestId("draft")).toHaveTextContent("typed draft");

    await signOut();
    await signIn(USER_B);

    await waitFor(() => expect(screen.getByTestId("draft")).toHaveTextContent(""));
  });
});
