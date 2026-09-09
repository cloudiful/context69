import { vi } from "vitest";

type Listener = (event: Event) => void;

/**
 * Minimal EventSource double for vitest. It records every constructed
 * instance (with its URL) and lets tests emit named SSE frames or force a
 * network-level failure, mirroring the subset of the API the search stream
 * composable relies on.
 */
export class FakeEventSource {
  static instances: FakeEventSource[] = [];
  static failNextConstructor = false;

  url: string;
  withCredentials: boolean;
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  private listeners = new Map<string, Listener[]>();
  private closed = false;

  constructor(url: string, options?: EventSourceInit) {
    this.url = url;
    this.withCredentials = options?.withCredentials ?? false;
    if (FakeEventSource.failNextConstructor) {
      FakeEventSource.failNextConstructor = false;
      throw new Error("EventSource unavailable");
    }
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: Listener): void {
    const group = this.listeners.get(type) ?? [];
    group.push(listener);
    this.listeners.set(type, group);
  }

  removeEventListener(type: string, listener: Listener): void {
    const group = this.listeners.get(type) ?? [];
    this.listeners.set(
      type,
      group.filter((entry) => entry !== listener),
    );
  }

  dispatchEvent(_event: Event): boolean {
    return true;
  }

  close(): void {
    this.closed = true;
    this.listeners.clear();
    this.onopen = null;
    this.onerror = null;
    this.onmessage = null;
  }

  get isClosed(): boolean {
    return this.closed;
  }

  /** Emit a named SSE frame with a JSON `data` payload. */
  emit(type: string, data: unknown): void {
    const frame = { data: JSON.stringify(data) } as unknown as Event;
    for (const listener of this.listeners.get(type) ?? []) {
      listener(frame);
    }
  }

  /** Force a network-level failure (EventSource `onerror`). */
  fail(): void {
    this.onerror?.(new Event("error"));
  }
}

/** Install the fake as the global EventSource and reset its state. */
export function installFakeEventSource(): typeof FakeEventSource {
  FakeEventSource.instances = [];
  FakeEventSource.failNextConstructor = false;
  vi.stubGlobal("EventSource", FakeEventSource);
  return FakeEventSource;
}

export function takeEventSource(previous?: FakeEventSource[]): FakeEventSource {
  const instance = FakeEventSource.instances.find((candidate) => !previous?.includes(candidate));
  if (!instance) {
    throw new Error("no EventSource instance was created");
  }
  return instance;
}
