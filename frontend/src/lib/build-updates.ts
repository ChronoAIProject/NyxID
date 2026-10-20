import { parseBuildVersion, type BuildVersion } from "./build-version";
export type { BuildVersion } from "./build-version";

const POLL_INTERVAL = 10 * 60_000;
const MAX_POLL_INTERVAL = 30 * 60_000;
const REQUEST_TIMEOUT = 15_000;
const RELOAD_GUARD = "nyxid_build_update_attempts";

interface BuildUpdateOptions {
  readonly buildId?: string;
  readonly pendingBuild?: BuildVersion;
  readonly fetch?: typeof fetch;
  readonly reload?: () => void;
  readonly canAutoReload: () => boolean;
  readonly onReady: (build: BuildVersion | null) => void;
  readonly onObserved?: (build: BuildVersion | null) => void;
}

/** Keep polling outside React; only an available update changes the UI. */
export function startBuildUpdates(options: BuildUpdateOptions) {
  const buildId = options.buildId ?? __BUILD_ID__;
  const fetcher = options.fetch ?? window.fetch.bind(window);
  const reload = options.reload ?? (() => window.location.reload());
  let stopped = false;
  let reloading = false;
  let inFlight = false;
  let lastCheck = -Infinity;
  let failures = 0;
  let candidate: BuildVersion | null = options.pendingBuild ?? null;
  let ready: BuildVersion | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let request: AbortController | null = null;
  let pendingCheck: Promise<void> | null = null;

  async function readVersion(signal: AbortSignal): Promise<BuildVersion | null> {
    // Health is a deployment hint. An unhealthy or independently rolled backend
    // cannot invalidate the frontend's own build identity.
    const health = fetcher("/health", {
      cache: "no-store", credentials: "omit", signal,
    }).then(async (response) => {
      if (response.ok) await response.json();
    }).catch(() => {});
    const response = await fetcher("/build-version.json", {
      cache: "no-store", credentials: "omit", signal,
    });
    if (!response.ok) throw new Error("Build metadata unavailable");
    // Frontend and backend images roll independently. The served SPA identity
    // is authoritative, including frontend-only deployments and rollbacks.
    const version = parseBuildVersion(await response.json());
    // Consume the health result when it settles; the frontend read can complete
    // even if the backend hangs. Cleanup aborts remaining requests.
    void health;
    return version;
  }

  async function stage(version: BuildVersion, signal: AbortSignal) {
    // Fetch without executing scripts or attaching new styles to the live page.
    let index = 0;
    await Promise.all(Array.from({ length: Math.min(4, version.assets.length) }, async () => {
      while (index < version.assets.length) {
        const asset = version.assets[index++]!;
        const response = await fetcher(`/${asset}`, { credentials: "omit", signal });
        const type = response.headers.get("content-type") ?? "";
        if (!response.ok || !(asset.endsWith(".css")
          ? type.includes("text/css")
          : /(?:javascript|ecmascript)/i.test(type))) {
          throw new Error("Build assets unavailable");
        }
        await response.arrayBuffer();
      }
    }));
    const response = await fetcher("/index.html", {
      cache: "no-store", credentials: "omit", signal,
    });
    if (!response.ok) throw new Error("App document unavailable");
    const document = new DOMParser().parseFromString(await response.text(), "text/html");
    if (document.querySelector('meta[name="nyxid-build-id"]')?.getAttribute("content") !== version.buildId) {
      throw new Error("App document changed");
    }
  }

  function announce(version: BuildVersion | null) {
    if (ready?.buildId === version?.buildId) return;
    ready = version;
    options.onReady(version);
  }

  function rememberAutoReload(version: BuildVersion) {
    try {
      const stored: unknown = JSON.parse(sessionStorage.getItem(RELOAD_GUARD) ?? "[]");
      if (!Array.isArray(stored) || !stored.every((id) => typeof id === "string")) return false;
      const attempts = new Set<string>([...stored, buildId]);
      if (attempts.has(version.buildId) || attempts.size >= 8) return false;
      attempts.add(version.buildId);
      sessionStorage.setItem(RELOAD_GUARD, JSON.stringify([...attempts]));
    } catch {
      return false;
    }
    return true;
  }

  function tryAutoReload() {
    if (!ready || stopped || reloading || !options.canAutoReload() || !rememberAutoReload(ready)) return;
    reloading = true;
    reload();
  }

  async function check() {
    if (stopped || reloading) return;
    if (inFlight || navigator.onLine === false) {
      timer = setTimeout(() => void check(), POLL_INTERVAL);
      return;
    }
    inFlight = true;
    let finishCheck = () => {};
    pendingCheck = new Promise<void>((resolve) => { finishCheck = resolve; });
    lastCheck = Date.now();
    const controller = new AbortController();
    request = controller;
    const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT);
    try {
      const version = await readVersion(controller.signal);
      if (stopped) return;
      failures = 0;
      options.onObserved?.(version);
      if (!version || version.buildId === buildId) {
        candidate = null;
        announce(null);
      } else if (candidate?.buildId === version.buildId && candidate.commit === version.commit) {
        if (ready?.buildId !== version.buildId) await stage(version, controller.signal);
        if (stopped) return;
        announce(version);
        tryAutoReload();
      } else {
        candidate = version;
        announce(null);
      }
    } catch {
      candidate = null;
      failures = Math.min(failures + 1, 3);
    } finally {
      clearTimeout(timeout);
      controller.abort();
      request = null;
      inFlight = false;
      pendingCheck = null;
      finishCheck();
      if (!stopped && !reloading) {
        clearTimeout(timer);
        timer = setTimeout(() => void check(), Math.min(POLL_INTERVAL * 2 ** failures, MAX_POLL_INTERVAL));
      }
    }
  }

  function wake() {
    // Never navigate on visibilitychange: returning users keep their current UI.
    if (document.visibilityState === "visible" && Date.now() - lastCheck >= POLL_INTERVAL) {
      clearTimeout(timer);
      void check();
    }
  }

  document.addEventListener("visibilitychange", wake);
  window.addEventListener("online", wake);
  void check();

  return {
    async apply(canApply?: () => boolean) {
      if (pendingCheck) await pendingCheck;
      if (!ready || stopped || reloading || inFlight) return false;
      // Explicit refresh revalidates after a notice has been left open for hours.
      const target = ready;
      const controller = new AbortController();
      request = controller;
      inFlight = true;
      const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT);
      try {
        const version = await readVersion(controller.signal);
        if (!version || version.buildId !== target.buildId) {
          announce(null);
          candidate = null;
          return false;
        }
        await stage(version, controller.signal);
        if (stopped || (canApply && (!canApply() || !rememberAutoReload(version)))) return false;
        reloading = true;
        reload();
        return true;
      } catch {
        announce(null);
        candidate = null;
        return false;
      } finally {
        clearTimeout(timeout);
        controller.abort();
        request = null;
        inFlight = false;
      }
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
      request?.abort();
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("online", wake);
    },
  };
}
