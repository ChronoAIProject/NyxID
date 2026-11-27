const WINDOW_MS = 10_000;
const THRESHOLD_MS = 50;
const TICK_MS = 250;

export type JsTaskTiming = { duration_ms: number; threshold_ms: number } & (
  | {
      observation_source: "rn_longtask";
      task_count: number;
      blocked_ms: number;
      max_task_ms?: number;
    }
  | {
      observation_source: "event_loop_lag";
      sample_count: number;
      tick_interval_ms: number;
      stall_count: number;
      max_lag_ms: number;
      lag_over_threshold_ms: number;
    }
);

interface Entry {
  startTime: number;
  duration: number;
}
interface Observer {
  observe(options: { type: "longtask"; buffered: false }): void;
  disconnect(): void;
  takeRecords(): Entry[];
}
interface ObserverConstructor {
  new (callback: (list: { getEntries(): Entry[] }) => void): Observer;
  supportedEntryTypes?: readonly string[];
}

function startWindow(
  emit: (result: JsTaskTiming) => void,
  now: () => number,
  readObserver: () => ObserverConstructor | undefined,
) {
  const start = now();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let observer: Observer | undefined;
  let closed = false;
  let count = 0;
  let max = 0;
  let excess = 0;
  function stop() {
    if (closed) return;
    closed = true;
    clearTimeout(timer);
    try {
      observer?.disconnect();
    } catch {
      /* The runtime may already be torn down. */
    }
  }
  function longTasks(entries: Entry[]) {
    if (closed || !observer) return;
    for (const entry of entries) {
      if (
        !Number.isFinite(entry.duration) ||
        entry.duration < THRESHOLD_MS ||
        !Number.isFinite(entry.startTime) ||
        entry.startTime < start
      )
        continue;
      count++;
      max = Math.max(max, entry.duration);
      excess += entry.duration - THRESHOLD_MS;
    }
  }
  try {
    // RN installs a lazy global getter which can also fail when loading native code.
    const ObserverType = readObserver();
    if (ObserverType?.supportedEntryTypes?.includes("longtask")) {
      observer = new ObserverType((list) => longTasks(list.getEntries()));
      observer.observe({ type: "longtask", buffered: false });
      timer = setTimeout(() => {
        // RN delivers entries asynchronously. Drain the native queue before disconnecting.
        try {
          longTasks(observer!.takeRecords());
          emit({
            observation_source: "rn_longtask",
            duration_ms: Math.round(now() - start),
            threshold_ms: THRESHOLD_MS,
            task_count: count,
            blocked_ms: Math.round(excess),
            ...(count ? { max_task_ms: Math.round(max) } : {}),
          });
        } catch {
          // An unavailable native timeline does not establish a zero-task window.
        } finally {
          stop();
        }
      }, WINDOW_MS);
      return stop;
    }
  } catch {
    try {
      observer?.disconnect();
    } catch {
      /* Proceed with the timer source. */
    }
    observer = undefined;
    count = max = excess = 0;
  }

  let ticks = 0;
  let last = start;
  function tick() {
    if (closed) return;
    const time = now();
    const lag = Math.max(0, time - last - TICK_MS);
    last = time;
    ticks++;
    max = Math.max(max, lag);
    if (lag >= THRESHOLD_MS) {
      count++;
      excess += lag - THRESHOLD_MS;
    }
    if (time - start < WINDOW_MS) {
      timer = setTimeout(tick, TICK_MS);
      return;
    }
    stop();
    emit({
      observation_source: "event_loop_lag",
      duration_ms: Math.round(time - start),
      threshold_ms: THRESHOLD_MS,
      tick_interval_ms: TICK_MS,
      sample_count: ticks,
      stall_count: count,
      max_lag_ms: Math.round(max),
      lag_over_threshold_ms: Math.round(excess),
    });
  }
  timer = setTimeout(tick, TICK_MS);
  return stop;
}

/** Sample once per uninterrupted active grant. Repeated lifecycle callbacks
 * cannot restart a completed or unsampled window. Dropped windows emit nothing. */
export function createJsTaskObserver(
  options: {
    active: () => boolean;
    current: () => number | null;
    begin: () => number | null;
    submit: (epoch: number, result: JsTaskTiming) => void;
  },
  now = () => performance.now(),
  observerType: () => ObserverConstructor | undefined = () =>
    (globalThis as { PerformanceObserver?: ObserverConstructor })
      .PerformanceObserver,
) {
  let activeGrant: number | null = null;
  let stop: (() => void) | undefined;
  function cancel() {
    stop?.();
    stop = undefined;
    activeGrant = null;
  }
  return {
    cancel,
    update() {
      const grant = options.active() ? options.current() : null;
      if (grant === activeGrant) return;
      cancel();
      activeGrant = grant;
      if (grant === null || options.begin() !== grant) return;
      stop = startWindow(
        (result) => {
          if (options.active() && options.current() === grant)
            options.submit(grant, result);
        },
        now,
        observerType,
      );
    },
  };
}
