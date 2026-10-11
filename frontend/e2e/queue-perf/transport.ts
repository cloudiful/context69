import type { BrowserContext, Route } from "@playwright/test";
import type { components } from "../../src/generated/openapi";
import { makeAuthMeResponse, makeTaskItemPage, makeTaskPage } from "./fixtures";

type TaskResponse = components["schemas"]["TaskResponse"];

interface QueueMockOptions {
  actionDelayMs?: number;
  failActionTaskId?: string;
}

export class QueueMockTransport {
  readonly unmatchedApiRequests: string[] = [];
  readonly blockedExternalRequests: string[] = [];
  readonly listQueries: Array<{ page: number; pageSize: number }> = [];
  readonly itemQueries: Array<{ taskId: string; cursor: string | null }> = [];
  readonly actionRequests: Array<{ kind: "retry" | "rerun"; taskId: string }> = [];
  private actionInFlight = 0;
  private maximumActionConcurrency = 0;
  private readonly tasks: TaskResponse[];
  private readonly actionDelayMs: number;
  private readonly failActionTaskId?: string;

  constructor(tasks: TaskResponse[], options: QueueMockOptions = {}) {
    this.tasks = tasks.map((task) => ({ ...task, progress: { ...task.progress } }));
    this.actionDelayMs = options.actionDelayMs ?? 0;
    this.failActionTaskId = options.failActionTaskId;
  }

  async install(context: BrowserContext, baseURL: string): Promise<void> {
    await context.addInitScript(installMockEventSource, this.tasks);
    const localOrigin = new URL(baseURL).origin;
    await context.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== localOrigin) {
        this.blockedExternalRequests.push(`${route.request().method()} ${url.origin}${url.pathname}`);
        await route.abort("blockedbyclient");
        return;
      }
      if (url.pathname === "/healthz" || url.pathname === "/v1" || url.pathname.startsWith("/v1/")) {
        const handled = await this.handleApiRoute(route, url);
        if (handled) return;
        this.unmatchedApiRequests.push(`${route.request().method()} ${url.pathname}`);
        await route.abort("blockedbyclient");
        return;
      }
      await route.continue();
    });
  }

  private async handleApiRoute(route: Route, url: URL): Promise<boolean> {
    const request = route.request();
    const method = request.method();
    if (method === "GET" && url.pathname === "/v1/auth/me") {
      await this.fulfill(route, makeAuthMeResponse());
      return true;
    }
    if (method === "GET" && url.pathname === "/v1/tasks") {
      const page = Number(url.searchParams.get("page") ?? 1);
      const pageSize = Number(url.searchParams.get("page_size") ?? 25);
      this.listQueries.push({ page, pageSize });
      await this.fulfill(route, makeTaskPage(this.tasks, page, pageSize));
      return true;
    }
    const itemsMatch = /^\/v1\/tasks\/([^/]+)\/items$/.exec(url.pathname);
    if (method === "GET" && itemsMatch) {
      const taskId = decodeURIComponent(itemsMatch[1]);
      const cursor = url.searchParams.get("cursor");
      this.itemQueries.push({ taskId, cursor });
      await this.fulfill(route, makeTaskItemPage(taskId, cursor === "next-100" ? 100 : 0));
      return true;
    }
    const actionMatch = /^\/v1\/tasks\/([^/]+)\/(retry|rerun)$/.exec(url.pathname);
    if (method === "POST" && actionMatch) {
      await this.handleRecovery(route, decodeURIComponent(actionMatch[1]), actionMatch[2] as "retry" | "rerun");
      return true;
    }
    return false;
  }

  private async handleRecovery(route: Route, taskId: string, kind: "retry" | "rerun"): Promise<void> {
    this.actionRequests.push({ kind, taskId });
    this.actionInFlight += 1;
    this.maximumActionConcurrency = Math.max(this.maximumActionConcurrency, this.actionInFlight);
    try {
      if (this.actionDelayMs > 0) await new Promise((resolve) => setTimeout(resolve, this.actionDelayMs));
      if (taskId === this.failActionTaskId) {
        await route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({ code: "mock_failure", message: "Synthetic recovery failure" }),
        });
        return;
      }
      const task = this.tasks.find((candidate) => candidate.task_id === taskId);
      if (!task) {
        await route.fulfill({ status: 404, contentType: "application/json", body: "{}" });
        return;
      }
      task.status = "queued";
      task.updated_at = new Date().toISOString();
      task.progress = { ...task.progress, cancelled: 0, failed: 0, queued: 1, running: 0 };
      const response = kind === "retry"
        ? { retried_items: 1, task: { task_id: taskId, item_ids: [] } }
        : { task: { task_id: taskId, item_ids: [] } };
      await this.fulfill(route, response);
    } finally {
      this.actionInFlight -= 1;
    }
  }

  private async fulfill(route: Route, body: unknown): Promise<void> {
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) });
  }

  snapshot() {
    return {
      listRequests: this.listQueries.length,
      listQueries: [...this.listQueries],
      itemRequests: this.itemQueries.length,
      itemQueries: [...this.itemQueries],
      retryRequests: this.actionRequests.filter(({ kind }) => kind === "retry").length,
      rerunRequests: this.actionRequests.filter(({ kind }) => kind === "rerun").length,
      actionRequests: [...this.actionRequests],
      maximumActionConcurrency: this.maximumActionConcurrency,
      unmatchedApiRequests: [...this.unmatchedApiRequests],
      blockedExternalRequests: [...this.blockedExternalRequests],
    };
  }
}

function installMockEventSource(initialTasks: TaskResponse[]) {
  type Listener = EventListenerOrEventListenerObject;
  class MockEventSource {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSED = 2;
    readonly url: string;
    readonly withCredentials = true;
    readyState = MockEventSource.CONNECTING;
    onopen: ((event: Event) => void) | null = null;
    onerror: ((event: Event) => void) | null = null;
    onmessage: ((event: MessageEvent) => void) | null = null;
    private readonly listeners = new Map<string, Set<Listener>>();

    constructor(url: string | URL) {
      this.url = String(url);
      registry.instances.push(this);
      setTimeout(() => {
        if (this.readyState === MockEventSource.CLOSED) return;
        this.readyState = MockEventSource.OPEN;
        this.dispatchEvent(new Event("open"));
        this.emit("snapshot", { tasks: initialTasks });
      }, 0);
    }

    addEventListener(type: string, listener: Listener): void {
      const listeners = this.listeners.get(type) ?? new Set<Listener>();
      listeners.add(listener);
      this.listeners.set(type, listeners);
    }

    removeEventListener(type: string, listener: Listener): void {
      this.listeners.get(type)?.delete(listener);
    }

    dispatchEvent(event: Event): boolean {
      for (const listener of this.listeners.get(event.type) ?? []) {
        if (typeof listener === "function") listener.call(this, event);
        else listener.handleEvent(event);
      }
      if (event.type === "open") this.onopen?.(event);
      if (event.type === "error") this.onerror?.(event);
      if (event.type === "message") this.onmessage?.(event as MessageEvent);
      return !event.defaultPrevented;
    }

    emit(type: string, payload: unknown): void {
      if (this.readyState !== MockEventSource.CLOSED) {
        this.dispatchEvent(new MessageEvent(type, { data: JSON.stringify(payload) }));
      }
    }

    close(): void {
      this.readyState = MockEventSource.CLOSED;
    }
  }

  const registry = {
    instances: [] as MockEventSource[],
    updatesEmitted: 0,
    emit(type: string, payload: unknown) {
      if (type === "update") this.updatesEmitted += 1;
      for (const source of this.instances) source.emit(type, payload);
    },
  };
  Object.defineProperty(window, "__queueEventSource", { configurable: true, value: registry });
  Object.defineProperty(window, "EventSource", { configurable: true, value: MockEventSource });
}
