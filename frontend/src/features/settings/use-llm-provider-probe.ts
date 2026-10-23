import { useCallback, useEffect, useRef, useState } from "react";

import { apiClient } from "@/services/api";
import type { TestLlmProviderRequest, TestLlmProviderResponse } from "@/services/api";

/**
 * Local probe state for the translation provider dialog. A newer probe, a
 * closed dialog, or a changed form field aborts the in-flight call and clears
 * the result; only the latest sequence may publish an outcome. The request body
 * is captured immutably when the button is clicked.
 */
export function useLlmProviderProbe() {
  const [pending, setPending] = useState(false);
  const [result, setResult] = useState<TestLlmProviderResponse | null>(null);
  const controller = useRef<AbortController | undefined>(undefined);
  const sequence = useRef(0);

  const reset = useCallback(() => {
    sequence.current += 1;
    controller.current?.abort();
    setPending(false);
    setResult(null);
  }, []);

  const submit = useCallback(async (payload: TestLlmProviderRequest) => {
    controller.current?.abort();
    controller.current = new AbortController();
    const current = ++sequence.current;
    setPending(true);
    const body: TestLlmProviderRequest = { ...payload };
    try {
      const response = await apiClient.testLlmProviderConnection(body, {
        signal: controller.current.signal,
      });
      if (current !== sequence.current) {
        return;
      }
      setResult(response);
    } finally {
      if (current === sequence.current) {
        setPending(false);
      }
    }
  }, []);

  useEffect(() => () => {
    sequence.current += 1;
    controller.current?.abort();
  }, []);

  return { pending, result, reset, submit };
}
