import { onBeforeUnmount, onMounted, ref, watch, type Ref } from "vue";

import { createTaskItemsAutomaticRefresh } from "./task-items-automatic-refresh";

export const ITEM_REFRESH_INTERVAL_MS = 3000;

export interface TaskItemsRequestOptions {
  /** Whether the caller still owns the current task/filter view. */
  isCurrent?: () => boolean;
  /** Drop this request instead of waiting when another item fetch is active. */
  dropIfBusy?: boolean;
}

export interface UseTaskItemsRefreshOptions {
  /** Task currently displayed; a change invalidates every in-flight response. */
  taskId: Ref<string>;
  /** Whether periodic reloading applies at all (mounted and active). */
  enabled: Ref<boolean>;
  /** Performs one periodic refresh. Failures preserve the last valid data. */
  refresh: () => Promise<void>;
  intervalMs?: number;
}

export interface UseTaskItemsRefresh {
  /** True while any initial, pagination, manual, or periodic fetch is active. */
  isRefreshing: Ref<boolean>;
  /** True when a periodic reload failed; last valid data is still shown. */
  refreshError: Ref<boolean>;
  /** Run one item fetch without overlapping any other entrypoint. */
  runExclusive: (
    request: () => Promise<void>,
    options?: TaskItemsRequestOptions,
  ) => Promise<boolean>;
  /** Keep only one trailing automatic refresh while a request is active. */
  scheduleAutomaticRefresh: (
    request: () => Promise<void>,
    options?: TaskItemsRequestOptions,
  ) => void;
  /** Invalidate a queued automatic refresh when its task/filter view changes. */
  invalidateAutomaticRefresh: () => void;
  start: () => void;
  stop: () => void;
}

export function useTaskItemsRefresh(options: UseTaskItemsRefreshOptions): UseTaskItemsRefresh {
  const isRefreshing = ref(false);
  const refreshError = ref(false);
  let timer: ReturnType<typeof setInterval> | null = null;
  let inFlight: Promise<void> | null = null;
  let generation = 0;
  let queuedExclusiveRequests = 0;
  let disposed = false;

  async function runExclusive(
    request: () => Promise<void>,
    requestOptions: TaskItemsRequestOptions = {},
  ): Promise<boolean> {
    const isCallerCurrent = requestOptions.isCurrent ?? (() => true);
    const isCurrent = () => !disposed && isCallerCurrent();
    if (!isCurrent()) return false;

    if (inFlight) {
      if (requestOptions.dropIfBusy) return false;
      const previous = inFlight;
      queuedExclusiveRequests += 1;
      try {
        await previous.catch(() => undefined);
      } finally {
        queuedExclusiveRequests -= 1;
      }
      if (!isCurrent()) {
        if (queuedExclusiveRequests === 0) automaticRefreshScheduler.onIdle();
        return false;
      }
      return runExclusive(request, requestOptions);
    }

    const requestGeneration = generation;
    let currentRequest: Promise<void>;
    currentRequest = Promise.resolve()
      .then(request)
      .finally(() => {
        if (inFlight === currentRequest) {
          inFlight = null;
          isRefreshing.value = false;
          automaticRefreshScheduler.onIdle();
        }
      });
    inFlight = currentRequest;
    isRefreshing.value = true;

    await currentRequest;
    return requestGeneration === generation && isCurrent();
  }

  const automaticRefreshScheduler = createTaskItemsAutomaticRefresh({
    isDisposed: () => disposed,
    isBusy: () => inFlight !== null || queuedExclusiveRequests > 0,
    run: (request, isCurrent) => runExclusive(request, { isCurrent }),
    onSuccess: () => { refreshError.value = false; },
    onError: () => { refreshError.value = true; },
  });

  function scheduleAutomaticRefresh(
    request: () => Promise<void>,
    requestOptions: TaskItemsRequestOptions = {},
  ) {
    automaticRefreshScheduler.schedule(request, requestOptions.isCurrent);
  }

  function invalidateAutomaticRefresh() {
    automaticRefreshScheduler.invalidate();
  }

  async function tick() {
    if (disposed || !options.enabled.value) return;
    const tickGeneration = generation;
    try {
      const completed = await runExclusive(options.refresh, {
        dropIfBusy: true,
        isCurrent: () => tickGeneration === generation && options.enabled.value,
      });
      if (completed && tickGeneration === generation) refreshError.value = false;
    } catch {
      if (tickGeneration === generation) refreshError.value = true;
    }
  }

  function stop() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
    generation += 1;
    invalidateAutomaticRefresh();
  }

  function start() {
    stop();
    if (disposed || !options.enabled.value) return;
    timer = setInterval(() => void tick(), options.intervalMs ?? ITEM_REFRESH_INTERVAL_MS);
  }

  watch(
    () => options.taskId.value,
    () => {
      generation += 1;
      invalidateAutomaticRefresh();
      refreshError.value = false;
    },
  );

  watch(options.enabled, (enabled) => {
    if (enabled) start();
    else stop();
  });

  onMounted(() => {
    if (options.enabled.value) start();
  });

  onBeforeUnmount(() => {
    disposed = true;
    stop();
  });

  return {
    isRefreshing,
    refreshError,
    runExclusive,
    scheduleAutomaticRefresh,
    invalidateAutomaticRefresh,
    start,
    stop,
  };
}
