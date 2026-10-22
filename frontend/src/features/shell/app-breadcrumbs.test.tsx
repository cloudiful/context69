import { render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { describe, expect, it } from "vitest";

import { AppProviders } from "@/app/providers";
import { AppBreadcrumbs, breadcrumbTrail, decodePathSegment } from "./app-breadcrumbs";

function renderAt(pathname: string) {
  const router = createMemoryRouter(
    [{ path: "*", element: <AppBreadcrumbs /> }],
    { initialEntries: [pathname] },
  );
  const { container } = render(
    <AppProviders>
      <RouterProvider router={router} />
    </AppProviders>,
  );
  const navigation = container.querySelector("nav") as HTMLElement;
  const links = Array.from(navigation.querySelectorAll("a")).map((link) => ({
    text: link.textContent ?? "",
    href: link.getAttribute("href") ?? "",
  }));
  return { navigation, links };
}

describe("app breadcrumbs", () => {
  it("keeps an encoded nested group as one decoded label and links to the encoded path", () => {
    const { navigation, links } = renderAt("/groups/stock%2Falpha/members");

    expect(navigation).toHaveTextContent("stock/alpha");
    expect(links).toContainEqual({ text: "Browser", href: "/groups" });
    expect(links).toContainEqual({ text: "stock/alpha", href: "/groups/stock%2Falpha" });
    expect(navigation).not.toHaveTextContent("stock%2Falpha");
  });

  it("keeps spaces and unicode readable while preserving the encoded hrefs", () => {
    const { navigation, links } = renderAt("/groups/%E4%B8%AD%E6%96%87%20group/members");

    expect(navigation).toHaveTextContent("中文 group");
    expect(links).toContainEqual({
      text: "中文 group",
      href: "/groups/%E4%B8%AD%E6%96%87%20group",
    });
    expect(navigation).toHaveTextContent("members");
  });

  it("renders a literal percent once and never double-encodes", () => {
    const { navigation, links } = renderAt("/groups/stock%2F100%25/members");

    expect(navigation).toHaveTextContent("stock/100%");
    expect(links).toContainEqual({ text: "stock/100%", href: "/groups/stock%2F100%25" });
  });

  it("keeps a malformed percent escape visible instead of throwing", () => {
    const { navigation, links } = renderAt("/groups/stock%2F100%/members");

    expect(navigation).toHaveTextContent("stock%2F100%");
    expect(links).toContainEqual({ text: "stock%2F100%", href: "/groups/stock%2F100%" });
  });

  it("keeps ordinary settings and document breadcrumbs labelled", () => {
    const settings = renderAt("/settings/appearance");
    expect(settings.navigation).toHaveTextContent("Settings");
    expect(settings.navigation).toHaveTextContent("Appearance");
    expect(settings.links).toContainEqual({ text: "Settings", href: "/settings" });

    const documents = renderAt("/documents/42");
    expect(documents.navigation).toHaveTextContent("Document Details");
    expect(documents.navigation).toHaveTextContent("42");
    // A section root without an index route stays a label, not a dead link.
    expect(documents.links).not.toContainEqual({ text: "Document Details", href: "/documents" });
  });

  it("decodes a segment without letting a bad escape escape the helper", () => {
    expect(decodePathSegment("stock%2Falpha")).toBe("stock/alpha");
    expect(decodePathSegment("100%")).toBe("100%");
    expect(breadcrumbTrail("/groups/a%2Fb/c")).toEqual([
      { segment: "groups", href: "/groups" },
      { segment: "a%2Fb", href: "/groups/a%2Fb" },
      { segment: "c", href: "/groups/a%2Fb/c" },
    ]);
  });
});
