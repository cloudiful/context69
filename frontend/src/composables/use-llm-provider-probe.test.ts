import { describe, expect, it, vi } from "vitest";

import { apiClient } from "../services/api";
import { useLlmProviderProbe } from "./use-llm-provider-probe";

vi.mock("../services/api", () => ({
  apiClient: { testLlmProviderConnection: vi.fn() },
}));

const probe = vi.mocked(apiClient.testLlmProviderConnection);

function payload() {
  return {
    endpoint: "https://api.openai.com/v1",
    model: "gpt-4o-mini",
    llm_api_kind: "openai_responses",
  };
}

describe("useLlmProviderProbe", () => {
  it("captures the clicked payload and stores the typed result", async () => {
    probe.mockResolvedValueOnce({ outcome: "success" } as never);
    const state = useLlmProviderProbe();

    await state.submitLlmProbe(payload());

    expect(probe).toHaveBeenCalledTimes(1);
    expect(probe.mock.calls[0][0]).toEqual(payload());
    expect(state.probeResult.value).toEqual({ outcome: "success" });
    expect(state.probePending.value).toBe(false);
  });

  it("ignores a stale completion after a newer probe starts", async () => {
    let first!: (value: unknown) => void;
    probe.mockImplementationOnce(() => new Promise((resolve) => { first = resolve as (value: unknown) => void; }));
    probe.mockResolvedValueOnce({ outcome: "timeout" } as never);
    const state = useLlmProviderProbe();

    const pending = state.submitLlmProbe(payload());
    await state.submitLlmProbe({ ...payload(), model: "other" });
    first({ outcome: "success" });
    await pending;

    expect(state.probeResult.value).toEqual({ outcome: "timeout" });
    expect(state.probePending.value).toBe(false);
  });

  it("clears the result and stops pending work on reset", async () => {
    probe.mockResolvedValueOnce({ outcome: "success" } as never);
    const state = useLlmProviderProbe();

    await state.submitLlmProbe(payload());
    expect(state.probeResult.value).not.toBeNull();

    state.resetProbe();

    expect(state.probeResult.value).toBeNull();
    expect(state.probePending.value).toBe(false);
  });
});
