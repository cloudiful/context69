import { onBeforeUnmount, onMounted, ref, watch, type Ref } from "vue";

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
  start: () => void;
  stop: () => void;
}

export function useTaskItemsRefresh(options: UseTaskItemsRefreshOptions): UseTaskItemsRefresh {
  const isRefreshing = ref(false);
  const refreshError = ref(false);
  let timer: ReturnType<typeof setInterval> | null = null;
  let inFlight: Promise<void> | null = null;
  let generation = 0;
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
      await previous.catch(() => undefined);
      if (!isCurrent()) return false;
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
        }
      });
    inFlight = currentRequest;
    isRefreshing.value = true;

    await currentRequest;
    return requestGeneration === generation && isCurrent();
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

  return { isRefreshing, refreshError, runExclusive, start, stop };
}
