export type LaunchKind = "cold" | "hot";
export type InputTarget = "login_email" | "chat_composer";
export type LaunchTiming = {
  startup_kind: LaunchKind;
  milestone: "app_active" | "first_frame" | "interaction_ready";
  target: "application" | InputTarget;
  origin_source: string;
  measurement_source: string;
  duration_ms: number;
};
export type LaunchSnapshot = {
  launch_id: number;
  startup_kind: LaunchKind;
  continuation_only?: boolean;
  origin_source: string;
  app_active_ms?: number;
  active_source?: string;
  first_frame_ms?: number;
  first_frame_source?: string;
};
export interface LaunchBridge {
  getLaunchSnapshot?(): Promise<LaunchSnapshot | null>;
  measureLaunchMilestone?(id: number): Promise<number | null>;
  setLaunchConsent?(binding: string | null): Promise<void>;
}
type Cycle = {
  kind: LaunchKind;
  epoch: number;
  id?: number;
  origin?: string;
  readyPending: boolean;
  route?: string;
  routeChanged: boolean;
  reported: Set<LaunchTiming["milestone"]>;
};
const validTime = (value: unknown): value is number =>
  typeof value === "number" &&
  Number.isFinite(value) &&
  value >= 0 &&
  value <= 120000;

// Native strings are projected to closed, platform-specific values before transmission.
const sources = {
  cold: ["ios_did_finish_launching", "android_process_uptime"],
  hot: [
    "ios_will_enter_foreground",
    "android_activity_started_after_background",
  ],
  app_active: ["ios_did_become_active", "android_activity_resumed"],
  first_frame: [
    "ios_window_commit_then_display_link_proxy",
    "android_window_frame_metrics_callback",
  ],
};

function matchesLaunchKind(snapshot: LaunchSnapshot, kind: LaunchKind) {
  return (
    snapshot.startup_kind === kind &&
    sources[kind].includes(snapshot.origin_source) &&
    Number.isSafeInteger(snapshot.launch_id) &&
    snapshot.launch_id >= 0
  );
}

export function createLaunchMonitor(deps: {
  native: LaunchBridge | null;
  androidActivityPause?: boolean;
  current: () => number | null;
  begin: () => number | null;
  coldGrant: () => Promise<number | null>;
  submit: (epoch: number, value: LaunchTiming) => void;
}) {
  let state: string | null = null;
  let cold = true;
  let background = false;
  let backgroundEpoch: number | null = null;
  let generation = 0;
  let cycle: Cycle | undefined;
  let suspended: Cycle | undefined;
  let lastId = -1;
  let poll: ReturnType<typeof setTimeout> | undefined;
  let expiry: ReturnType<typeof setTimeout> | undefined;
  let inputRetry: ReturnType<typeof setTimeout> | undefined;
  let splashHidden = false;
  let route: string | undefined;
  let clear = false;
  const inputs = new Map<
    object,
    {
      target: InputTarget;
      eligible: () => boolean;
      visible: () => Promise<boolean>;
    }
  >();

  function valid(c: Cycle) {
    return cycle === c && state === "active" && c.epoch === deps.current();
  }
  function reset() {
    generation++;
    clearTimeout(poll);
    clearTimeout(expiry);
    clearTimeout(inputRetry);
    cycle = undefined;
    suspended = undefined;
  }
  function send(
    c: Cycle,
    milestone: LaunchTiming["milestone"],
    target: LaunchTiming["target"],
    source: string,
    ms: unknown,
  ) {
    if (!valid(c) || !validTime(ms) || !c.origin || c.reported.has(milestone))
      return;
    c.reported.add(milestone);
    deps.submit(c.epoch, {
      startup_kind: c.kind,
      milestone,
      target,
      origin_source: c.origin,
      measurement_source: source,
      duration_ms: Math.round(ms),
    });
    if (c.reported.size === 3) reset();
  }
  function ready(attempt = 0) {
    const c = cycle;
    if (
      !c ||
      !valid(c) ||
      c.id === undefined ||
      !splashHidden ||
      !route ||
      !clear ||
      c.routeChanged ||
      c.readyPending ||
      c.reported.has("interaction_ready") ||
      !deps.native?.measureLaunchMilestone
    )
      return;
    const entry = [...inputs.entries()].find(([, row]) => row.eligible());
    if (!entry) return;
    const [inputKey, candidate] = entry;
    const key = route;
    const isCurrentInput = () =>
      valid(c) &&
      key === route &&
      clear &&
      !c.routeChanged &&
      inputs.get(inputKey) === candidate &&
      candidate.eligible();
    clearTimeout(inputRetry);
    c.readyPending = true;
    const id = c.id;
    void candidate
      .visible()
      .then((visible) => {
        if (!visible || !isCurrentInput()) return null;
        return deps.native?.measureLaunchMilestone?.(id);
      })
      .then((duration) => {
        if (isCurrentInput())
          send(
            c,
            "interaction_ready",
            candidate.target,
            "enabled_input_in_visible_window",
            duration,
          );
      })
      .catch(() => {})
      .finally(() => {
        c.readyPending = false;
        if (!valid(c) || c.reported.has("interaction_ready")) return;
        if (inputs.get(inputKey) !== candidate) {
          ready();
        } else if (isCurrentInput() && attempt < 4) {
          // First window measurement can run before a navigation animation
          // settles. Briefly retry an eligible node even without a new layout.
          inputRetry = setTimeout(() => ready(attempt + 1), 100 * 2 ** attempt);
        }
      });
  }
  async function read(c: Cycle, attempt = 0, firstSnapshot?: LaunchSnapshot) {
    if (!valid(c)) return;
    const snap =
      firstSnapshot ?? (await deps.native?.getLaunchSnapshot?.().catch(() => null));
    if (!valid(c)) return;
    if (
      snap &&
      matchesLaunchKind(snap, c.kind) &&
      (c.id === undefined
        ? !snap.continuation_only && snap.launch_id > lastId
        : c.id === snap.launch_id)
    ) {
      c.id = snap.launch_id;
      lastId = c.id;
      c.origin = snap.origin_source;
      if (snap.active_source && sources.app_active.includes(snap.active_source))
        send(
          c,
          "app_active",
          "application",
          snap.active_source,
          snap.app_active_ms,
        );
      if (
        snap.first_frame_source &&
        sources.first_frame.includes(snap.first_frame_source)
      )
        send(
          c,
          "first_frame",
          "application",
          snap.first_frame_source,
          snap.first_frame_ms,
        );
      ready();
    }
    if (
      valid(c) &&
      (c.id !== undefined || attempt < 20) &&
      (!c.reported.has("app_active") || !c.reported.has("first_frame"))
    )
      poll = setTimeout(
        () => {
          void read(c, attempt + 1);
        },
        attempt < 20 ? 150 : 1000,
      );
  }
  function start(kind: LaunchKind, grant: number | null, snapshot?: LaunchSnapshot) {
    if (
      !deps.native?.getLaunchSnapshot ||
      grant === null ||
      grant !== deps.current() ||
      deps.begin() !== grant
    )
      return;
    cycle = {
      kind,
      epoch: grant,
      readyPending: false,
      route,
      routeChanged: false,
      reported: new Set(),
    };
    expiry = setTimeout(reset, 120000);
    void read(cycle, 0, snapshot);
  }
  async function resumeAndroid(grant: number, gen: number, attempt = 0) {
    const current = () =>
      generation === gen && state === "active" && deps.current() === grant;
    if (!current()) return;
    const snapshot = await deps.native?.getLaunchSnapshot?.().catch(() => null);
    if (!current()) return;
    if (!snapshot) {
      if (attempt < 20)
        poll = setTimeout(() => void resumeAndroid(grant, gen, attempt + 1), 150);
      return;
    }
    const previous = suspended;
    suspended = undefined;
    if (!snapshot.continuation_only) {
      if (matchesLaunchKind(snapshot, "hot") && snapshot.launch_id > lastId)
        start("hot", grant, snapshot);
      return;
    }
    // RN Android reports background on Activity.pause, including a short
    // permission sheet. Hot samples already have the ID from resumeAndroid.
    // The process has only one cold launch, which may pause before its first read.
    if (
      !previous ||
      previous.epoch !== grant ||
      !matchesLaunchKind(snapshot, previous.kind) ||
      (previous.id === undefined
        ? previous.kind !== "cold" || snapshot.launch_id <= lastId
        : previous.id !== snapshot.launch_id)
    )
      return;
    cycle = { ...previous, id: snapshot.launch_id, readyPending: false };
    expiry = setTimeout(reset, 120000);
    void read(cycle, 0, snapshot);
  }
  return {
    state(next: string | null) {
      if (next === state) return;
      state = next;
      if (next === "background") {
        cold = false;
        background = true;
        backgroundEpoch = deps.current();
        const previous = deps.androidActivityPause ? cycle ?? suspended : undefined;
        reset();
        suspended = previous;
        return;
      }
      if (next !== "active") {
        if (cycle) reset();
        return;
      }
      if (background) {
        background = false;
        if (
          deps.androidActivityPause &&
          backgroundEpoch !== null &&
          backgroundEpoch === deps.current() &&
          deps.native?.getLaunchSnapshot
        ) {
          void resumeAndroid(backgroundEpoch, generation);
        } else {
          start("hot", backgroundEpoch);
        }
      } else if (cold) {
        cold = false;
        const gen = generation;
        void deps
          .coldGrant()
          .then((grant) => {
            if (gen === generation && state === "active") start("cold", grant);
          })
          .catch(() => {});
      }
    },
    consent() {
      if (
        (cycle && deps.current() !== cycle.epoch) ||
        (suspended && deps.current() !== suspended.epoch)
      )
        reset();
    },
    splash() {
      splashHidden = true;
      ready();
    },
    shell(key: string | undefined, interactive: boolean) {
      route = key;
      clear = interactive;
      const launch = cycle ?? suspended;
      if (launch && key) {
        if (launch.route && key !== launch.route) launch.routeChanged = true;
        launch.route ??= key;
      }
      ready();
    },
    input(
      key: object,
      target: InputTarget,
      eligible: () => boolean,
      visible: () => Promise<boolean>,
    ) {
      const row = { target, eligible, visible };
      inputs.set(key, row);
      ready();
      return () => {
        if (inputs.get(key) === row) inputs.delete(key);
      };
    },
    reset,
  };
}
