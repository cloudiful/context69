import { render, screen } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";

import { StatusBadge, type StatusTone } from "./status-badge";

const TONES: StatusTone[] = ["neutral", "active", "info", "success", "warning", "failure"];

describe("status badge", () => {
  it("renders every tone with its token classes and a data-tone hook", () => {
    const { container } = render(
      <>
        {TONES.map((tone) => (
          <StatusBadge key={tone} tone={tone}>{tone}</StatusBadge>
        ))}
      </>,
    );

    for (const tone of TONES) {
      const badge = container.querySelector(`[data-tone="${tone}"]`);
      expect(badge).not.toBeNull();
      expect(badge).toHaveClass(`border-status-${tone}-border`);
      expect(badge).toHaveClass(`bg-status-${tone}-surface`);
      expect(badge).toHaveClass(`text-status-${tone}-foreground`);
    }
  });

  it("defaults to the neutral tone without exposing an alternate variant", () => {
    render(<StatusBadge>Unknown</StatusBadge>);

    const badge = screen.getByText("Unknown");
    expect(badge).toHaveAttribute("data-tone", "neutral");
    expect(badge).toHaveAttribute("data-slot", "badge");
    expect(badge).toHaveAttribute("data-variant", "outline");
  });

  it("keeps the label children and composes an extra className", () => {
    render(<StatusBadge tone="success" className="font-mono">Succeeded</StatusBadge>);

    const badge = screen.getByText("Succeeded");
    expect(badge).toHaveTextContent("Succeeded");
    expect(badge).toHaveClass("font-mono");
    expect(badge).toHaveClass("bg-status-success-surface");
  });

  it("forwards native span props and the ref", () => {
    const ref = createRef<HTMLSpanElement>();
    const onClick = vi.fn();
    render(
      <StatusBadge ref={ref} tone="failure" title="Failed" aria-label="task failed" onClick={onClick}>
        Failed
      </StatusBadge>,
    );

    const badge = screen.getByLabelText("task failed");
    expect(badge).toHaveAttribute("title", "Failed");
    expect(ref.current).toBe(badge);

    badge.click();
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});
