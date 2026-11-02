import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it, vi } from "vitest";

import { i18n } from "@/lib/i18n";

import { QueueBulkReport } from "./queue-bulk-report";
import { QUEUE_BULK_REPORT_PAGE_SIZE, type QueueBulkReport as QueueBulkReportModel } from "./queue-helpers";

function report(count: number): QueueBulkReportModel {
  return {
    action: "retry",
    summary: { succeeded: count, skipped: 0, failed: 0, partial: 0 },
    entries: Array.from({ length: count }, (_, index) => ({
      taskId: `task-${index}`,
      subject: `Subject ${index}`,
      status: index % 2 === 0 ? "accepted" as const : "skipped" as const,
      targetTaskId: index % 2 === 0 ? `task-${index}` : null,
      reason: index % 2 === 0 ? null : "nothing_to_retry",
    })),
  };
}

function renderReport(model: QueueBulkReportModel, onDismiss = vi.fn(), onOpenTask = vi.fn()) {
  render(
    <I18nextProvider i18n={i18n}>
      <QueueBulkReport report={model} onDismiss={onDismiss} onOpenTask={onOpenTask} />
    </I18nextProvider>,
  );
  return { onDismiss, onOpenTask };
}

describe("QueueBulkReport", () => {
  it("renders one bounded page of rows and pages through the rest", async () => {
    const user = userEvent.setup();
    renderReport(report(QUEUE_BULK_REPORT_PAGE_SIZE * 2 + 20));

    expect(screen.getAllByTestId("queue-bulk-report-row")).toHaveLength(QUEUE_BULK_REPORT_PAGE_SIZE);
    expect(screen.getByText("Page 1 of 3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Previous result page" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Next result page" }));
    expect(screen.getAllByTestId("queue-bulk-report-row")).toHaveLength(QUEUE_BULK_REPORT_PAGE_SIZE);

    await user.click(screen.getByRole("button", { name: "Next result page" }));
    expect(screen.getAllByTestId("queue-bulk-report-row")).toHaveLength(20);
    expect(screen.getByText("Page 3 of 3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next result page" })).toBeDisabled();
  });

  it("keeps the subject and the per-item reason, and opens the accepted target", async () => {
    const user = userEvent.setup();
    const { onOpenTask } = renderReport(report(2));

    const firstRow = screen.getAllByTestId("queue-bulk-report-row")[0]!;
    expect(within(firstRow).getByText("Subject 0")).toBeInTheDocument();
    expect(within(firstRow).getByText("Accepted")).toBeInTheDocument();
    await user.click(within(firstRow).getByRole("button", { name: "Open Subject 0" }));
    expect(onOpenTask).toHaveBeenCalledWith("task-0");

    const secondRow = screen.getAllByTestId("queue-bulk-report-row")[1]!;
    expect(within(secondRow).getByText("nothing to retry")).toBeInTheDocument();
    expect(within(secondRow).getByText("Skipped")).toBeInTheDocument();
    expect(within(secondRow).queryByRole("button")).not.toBeInTheDocument();
  });

  it("labels a forked target as a new task and reports dismissal", async () => {
    const user = userEvent.setup();
    const { onDismiss, onOpenTask } = renderReport({
      action: "rerun",
      summary: { succeeded: 1, skipped: 0, failed: 0, partial: 0 },
      entries: [{
        taskId: "parent-1",
        subject: "Annual Report",
        status: "accepted",
        targetTaskId: "child-1",
        reason: null,
      }],
    });

    const button = screen.getByRole("button", { name: "Open Annual Report" });
    expect(button).toHaveTextContent("View new task");
    await user.click(button);
    expect(onOpenTask).toHaveBeenCalledWith("child-1");

    await user.click(screen.getByRole("button", { name: "Dismiss result" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
