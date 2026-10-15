import { onBeforeUnmount, ref } from "vue";

import { apiClient, type TestLlmProviderRequest, type TestLlmProviderResponse } from "../services/api";

/**
 * Local probe state for the LLM edit dialog (issue #90 P1).
 *
 * Owns only the probe request lifecycle: pending flag, typed result, one
 * AbortController, and a monotonic sequence id. A newer probe, a closed or
 * reopened dialog, a changed form field, or an unmount aborts the in-flight
 * call, clears the result, and ignores the stale completion — an abort never
 * guarantees the upstream call stops, so only the latest sequence may write.
 * The editing draft itself is never mutated here; the payload is captured
 * immutably when the button is clicked.
 */
export function useLlmProviderProbe() {
  const probePending = ref(false);
  const probeResult = ref<TestLlmProviderResponse | null>(null);
  let controller: AbortController | undefined;
  let sequence = 0;

  function clearProbeResult() {
    probeResult.value = null;
  }

  function abortProbe() {
    controller?.abort();
  }

  function resetProbe() {
    sequence += 1;
    abortProbe();
    probePending.value = false;
    clearProbeResult();
  }

  async function submitLlmProbe(payload: TestLlmProviderRequest) {
    abortProbe();
    controller = new AbortController();
    const current = ++sequence;
    probePending.value = true;
    const body: TestLlmProviderRequest = { ...payload };
    try {
      const result = await apiClient.testLlmProviderConnection(body, {
        signal: controller.signal,
      });
      if (current !== sequence) return;
      probeResult.value = result;
    } finally {
      if (current === sequence) probePending.value = false;
    }
  }

  onBeforeUnmount(() => {
    sequence += 1;
    abortProbe();
  });

  return { probePending, probeResult, submitLlmProbe, clearProbeResult, abortProbe, resetProbe };
}
