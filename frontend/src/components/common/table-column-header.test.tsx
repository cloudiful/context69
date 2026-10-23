import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { TableColumnHeader } from "./table-column-header";

describe("TableColumnHeader", () => {
  it("renders the title once with an optional independent sort action", async () => {
    const user = userEvent.setup();
    const onSort = vi.fn();
    render(
      <TableColumnHeader
        label="Status"
        sort={{ direction: "asc", onSort, ariaLabel: "Sort by Status, sorted ascending" }}
      />,
    );

    expect(screen.getAllByText("Status")).toHaveLength(1);
    const sort = screen.getByRole("button", { name: "Sort by Status, sorted ascending" });
    expect(sort).toHaveAttribute("data-sort", "asc");
    await user.click(sort);
    expect(onSort).toHaveBeenCalledOnce();
  });

  it("marks an unsorted column without repeating its title in the accessible name", () => {
    render(
      <TableColumnHeader
        label="Group"
        sort={{ direction: "none", onSort: () => {}, ariaLabel: "Sort by Group, not sorted" }}
      />,
    );

    expect(screen.getByRole("button", { name: "Sort by Group, not sorted" })).toHaveAttribute("data-sort", "none");
  });

  it("renders a filter slot as a separate action next to the title", () => {
    render(
      <TableColumnHeader
        label="Type"
        filter={<button type="button" aria-label="Filter by Type">filter</button>}
      />,
    );

    expect(screen.getAllByText("Type")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Filter by Type" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sort by Type" })).not.toBeInTheDocument();
  });
});
