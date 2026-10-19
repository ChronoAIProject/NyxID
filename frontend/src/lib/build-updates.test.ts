import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { startBuildUpdates, type BuildVersion } from "./build-updates";

const current = "111111111111";
const next = "222222222222";
const other = "333333333333";
const manifest = (id = next): BuildVersion => ({
  buildId: id, commit: id, assets: ["assets/app-next.js", "assets/app-next.css"],
});

let served: BuildVersion;
let healthCommit: string;
let htmlId: string;
let online: boolean;
let healthy: boolean;
let assetType: string;
let reload: ReturnType<typeof vi.fn<() => void>>;
let onReady: ReturnType<typeof vi.fn<(build: BuildVersion | null) => void>>;
let fetcher: ReturnType<typeof vi.fn<typeof fetch>>;
let updates: ReturnType<typeof startBuildUpdates> | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  sessionStorage.clear();
  served = manifest();
  healthCommit = next;
  htmlId = next;
  online = true;
  healthy = true;
  assetType = "text/javascript";
  reload = vi.fn();
  onReady = vi.fn();
  vi.spyOn(navigator, "onLine", "get").mockImplementation(() => online);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  fetcher = vi.fn(async (input) => {
    if (input === "/health") {
      return Response.json({ status: "ok", commit: healthCommit });
    }
    if (input === "/build-version.json") {
      if (!healthy) throw new Error("offline");
      return Response.json(served);
    }
    if (input === "/index.html") {
      return new Response(`<meta name="nyxid-build-id" content="${htmlId}">`);
    }
    return new Response("asset", { headers: {
      "content-type": String(input).endsWith(".css") ? "text/css" : assetType,
    } });
  });
});

afterEach(() => {
  updates?.stop();
  updates = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

async function start(canAutoReload = () => false) {
  updates = startBuildUpdates({ buildId: current, fetch: fetcher, reload, onReady, canAutoReload });
  await vi.advanceTimersByTimeAsync(0);
}

async function confirm() {
  await vi.advanceTimersByTimeAsync(600_000);
}

describe("background build updates", () => {
  it("confirms twice and stages assets without touching the visible app or reloading", async () => {
    document.body.innerHTML = '<div id="root"><input value="unsaved draft"></div>';
    const root = document.getElementById("root");
    await start();
    expect(onReady).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60_000);
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event("online"));
    await vi.advanceTimersByTimeAsync(539_999);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(onReady).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(onReady).toHaveBeenCalledExactlyOnceWith(manifest());
    expect(fetcher.mock.calls.map(([url]) => url)).toContain("/assets/app-next.js");
    expect(document.getElementById("root")).toBe(root);
    expect(document.querySelector("input")?.value).toBe("unsaved draft");
    expect(document.querySelector("script, link")).toBeNull();
    expect(reload).not.toHaveBeenCalled();
    for (const [url, config] of fetcher.mock.calls) {
      expect(config?.credentials).toBe("omit");
      if (url === "/health" || url === "/build-version.json" || url === "/index.html") {
        expect(config?.cache).toBe("no-store");
      }
    }
  });

  it("does nothing when the frontend still serves the current build after a backend deploy", async () => {
    served = manifest(current);
    await start();
    await confirm();
    expect(onReady).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
    expect(fetcher.mock.calls.some(([url]) => String(url).startsWith("/assets/"))).toBe(false);
  });

  it("detects a frontend-only deploy even with an unchanged backend commit", async () => {
    healthCommit = current;
    await start();
    await confirm();
    expect(onReady).toHaveBeenCalledWith(manifest());
  });

  it("uses served frontend metadata when health fails or reports an unknown commit", async () => {
    fetcher.mockImplementation(async (input) => {
      if (input === "/health") throw new Error("backend unavailable");
      if (input === "/build-version.json") return Response.json(manifest());
      if (input === "/index.html") return new Response(`<meta name="nyxid-build-id" content="${next}">`);
      return new Response("asset", { headers: { "content-type": String(input).endsWith(".css") ? "text/css" : "text/javascript" } });
    });
    await start();
    await confirm();
    expect(onReady).toHaveBeenCalledWith(manifest());
  });

  it("does not confirm alternating frontend replicas", async () => {
    await start();
    for (const id of [other, next, other, next]) {
      served = manifest(id);
      await confirm();
    }
    expect(onReady).not.toHaveBeenCalled();
  });

  it("cancels an update when the current build is served again", async () => {
    await start();
    await confirm();
    served = manifest(current);
    await confirm();
    expect(onReady).toHaveBeenLastCalledWith(null);
  });

  it.each(["wrong-document", "html-asset", "invalid-path"])("does not reload an incomplete build: %s", async (fault) => {
    if (fault === "wrong-document") htmlId = other;
    if (fault === "html-asset") assetType = "text/html";
    if (fault === "invalid-path") served = { ...served, assets: ["https://evil.example/app.js"] };
    await start(() => true);
    await confirm();
    expect(onReady).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
  });

  it("reloads only when the live automatic-update gate allows it", async () => {
    let safe = false;
    await start(() => safe);
    await confirm();
    expect(reload).not.toHaveBeenCalled();
    safe = true;
    await confirm();
    expect(reload).toHaveBeenCalledOnce();
  });

  it("bounds automatic reloads across boots that still land on the old image", async () => {
    await start(() => true);
    await confirm();
    expect(reload).toHaveBeenCalledOnce();
    updates?.stop();
    await start(() => true);
    await confirm();
    expect(reload).toHaveBeenCalledOnce();
  });

  it("keeps the current UI and ready notice on network failures, backs off and recovers", async () => {
    await start();
    await confirm();
    healthy = false;
    await confirm();
    expect(onReady).toHaveBeenCalledTimes(1);
    const count = fetcher.mock.calls.length;
    await confirm();
    expect(fetcher).toHaveBeenCalledTimes(count);
    healthy = true;
    await confirm();
    await confirm();
    expect(reload).not.toHaveBeenCalled();
  });

  it("resumes polling after an offline scheduled check", async () => {
    await start();
    online = false;
    await confirm();
    online = true;
    await confirm();
    expect(onReady).toHaveBeenCalledWith(manifest());
  });

  it("revalidates an explicit refresh and refuses a stale deployment notice", async () => {
    await start();
    await confirm();
    served = manifest(other);
    expect(await updates?.apply()).toBe(false);
    expect(reload).not.toHaveBeenCalled();
    expect(onReady).toHaveBeenLastCalledWith(null);
  });

  it("allows an explicit refresh after revalidating a ready build", async () => {
    await start();
    await confirm();
    expect(await updates?.apply()).toBe(true);
    expect(reload).toHaveBeenCalledOnce();
  });

  it("checks live safety after deployment revalidation before applying a view update", async () => {
    await start();
    await confirm();
    const gate = vi.fn(() => false);
    expect(await updates?.apply(gate)).toBe(false);
    expect(gate).toHaveBeenCalledOnce();
    expect(reload).not.toHaveBeenCalled();
    gate.mockReturnValue(true);
    expect(await updates?.apply(gate)).toBe(true);
    expect(reload).toHaveBeenCalledOnce();
    updates?.stop();
    await start();
    await confirm();
    expect(await updates?.apply(gate)).toBe(false);
    expect(reload).toHaveBeenCalledOnce();
    expect(await updates?.apply()).toBe(true);
    expect(reload).toHaveBeenCalledTimes(2);
  });

  it("revalidates and stages a persisted confirmed candidate at startup", async () => {
    updates = startBuildUpdates({ buildId: current, pendingBuild: manifest(), fetch: fetcher, reload, onReady, canAutoReload: () => false });
    await vi.advanceTimersByTimeAsync(0);
    expect(onReady).toHaveBeenCalledExactlyOnceWith(manifest());
    expect(reload).not.toHaveBeenCalled();
  });

  it("stops timers and aborts a pending poll on cleanup", async () => {
    fetcher.mockImplementation((_input, config) => new Promise((_resolve, reject) => {
      config?.signal?.addEventListener("abort", () => reject(new Error("aborted")));
    }));
    await start();
    const signal = fetcher.mock.calls[0]?.[1]?.signal;
    updates?.stop();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(signal?.aborted).toBe(true);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(onReady).not.toHaveBeenCalled();
  });
});
