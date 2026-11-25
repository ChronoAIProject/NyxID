import type {
  NativePerformance,
  PerformanceScreen,
  StartupKind,
  StartupTiming,
} from "./nativePerformance";

export function createStartupObserver(deps: {
  native: NativePerformance | null;
  launchEpoch: () => Promise<number | null>;
  begin: () => number | null;
  current: () => number | null;
  submit: (
    epoch: number,
    kind: StartupKind,
    screen: PerformanceScreen,
    timing: StartupTiming,
  ) => void;
}) {
  let active = true;
  let cycle = 0;
  let launchObserved = false;
  let pendingResume: { cycle: number; epoch: number | null } | undefined;
  let observed: PerformanceScreen | undefined;
  let wasBackground = false;
  const report = (
    kind: StartupKind,
    screen: PerformanceScreen,
    epoch: Promise<number | null>,
    eligible: () => boolean,
  ) => {
    if (!deps.native) return;
    const capturedCycle = cycle;
    const native = deps.native;
    const allowed = (consent: number | null): consent is number =>
      consent !== null &&
      consent === deps.current() &&
      active &&
      capturedCycle === cycle &&
      observed === screen &&
      eligible();
    // Native draw/display observers start ONLY after persisted launch authority.
    // The boundary includes authorization hydration if it finishes after layout.
    void epoch
      .then(async (consent) => {
        if (!allowed(consent)) return;
        const timing = await native.measureReady(kind).catch(() => null);
        if (!allowed(consent) || !timing) return;
        const expected =
          kind === "cold"
            ? ["android_process_uptime", "ios_did_finish_launching"]
            : ["android_activity_resume", "ios_will_enter_foreground"];
        if (
          !expected.includes(timing.startup_source) ||
          !Number.isFinite(timing.duration_ms) ||
          timing.duration_ms < 0 ||
          timing.duration_ms > 120000
        )
          return;
        deps.submit(consent, kind, screen, {
          duration_ms: timing.duration_ms,
          startup_source: timing.startup_source,
        });
      })
      .catch(() => {});
  };
  return {
    ready(screen: PerformanceScreen, eligible: () => boolean = () => true) {
      if (!active || !eligible()) return;
      observed = screen;
      if (!launchObserved) {
        launchObserved = true;
        report("cold", screen, deps.launchEpoch(), eligible);
      } else if (pendingResume?.cycle === cycle) {
        const { epoch } = pendingResume;
        pendingResume = undefined;
        report("resume", screen, Promise.resolve(epoch), eligible);
      }
    },
    foreground(next: boolean, backgrounded = !next) {
      // iOS inactive (Control Center/permission UI) is not a background resume.
      if (!next && backgrounded) {
        wasBackground = true;
        // A genuine background interrupts the launch; unknown/inactive initial
        // AppState must leave the first active readiness eligible for cold.
        launchObserved = true;
        pendingResume = undefined;
      }
      if (active === next) return;
      active = next;
      cycle++;
      observed = undefined;
      if (!next) {
        pendingResume = undefined;
      } else {
        pendingResume = wasBackground
          ? { cycle, epoch: deps.begin() }
          : undefined;
        wasBackground = false;
      }
    },
    invalidate() {
      cycle++;
      pendingResume = undefined;
      observed = undefined;
    },
  };
}
