interface AutomaticRefreshJob {
  request: () => Promise<void>;
  isCurrent: () => boolean;
}

export interface TaskItemsAutomaticRefreshOptions {
  isDisposed: () => boolean;
  isBusy: () => boolean;
  run: (request: () => Promise<void>, isCurrent: () => boolean) => Promise<boolean>;
  onSuccess: () => void;
  onError: () => void;
}

export function createTaskItemsAutomaticRefresh(options: TaskItemsAutomaticRefreshOptions) {
  let generation = 0;
  let running = false;
  let pending: AutomaticRefreshJob | null = null;

  async function drain() {
    if (running || options.isDisposed() || options.isBusy()) return;
    running = true;
    try {
      while (!options.isDisposed() && pending && !options.isBusy()) {
        const job = pending;
        pending = null;
        if (!job.isCurrent()) continue;
        try {
          const completed = await options.run(job.request, job.isCurrent);
          if (completed && job.isCurrent()) options.onSuccess();
        } catch {
          if (job.isCurrent()) options.onError();
        }
      }
    } finally {
      running = false;
      if (pending && !options.isDisposed() && !options.isBusy()) void drain();
    }
  }

  function schedule(request: () => Promise<void>, isCallerCurrent: () => boolean = () => true) {
    const requestGeneration = generation;
    const isCurrent = () => !options.isDisposed()
      && requestGeneration === generation
      && isCallerCurrent();
    if (!isCurrent()) return;

    pending = { request, isCurrent };
    if (!running && !options.isBusy()) void drain();
  }

  function invalidate() {
    generation += 1;
    pending = null;
  }

  function onIdle() {
    if (!running && pending && !options.isDisposed() && !options.isBusy()) void drain();
  }

  return { schedule, invalidate, onIdle };
}
