import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// `beforeEach` is used inside the installHeartbeat describe for fake timers.
import type { WizardBootstrap } from "./client";

// The shim is a module-level singleton (`installed` / `currentBootstrap`)
// that wraps `window.fetch` exactly once. To get a clean wrap per test we
// reset the module registry and re-import. Each helper below imports a
// freshly-evaluated copy of the module so the `installed` flag is false
// again.
async function freshClient() {
  vi.resetModules();
  return import("./client");
}

const BOOTSTRAP: WizardBootstrap = {
  flow: "ai-key",
  csrf: "csrf-token-xyz",
  baseUrl: "http://localhost:3001",
  context: "local",
};

// Capture the pristine `window.fetch` ONCE, before any test installs the
// shim. The shim mutates the live `window.fetch` (module-level singleton),
// so per-test save/restore must always rewind to this genuine original —
// otherwise a wrapped shim leaks out of this file and pollutes the global
// fetch seen by every other test file in the run.
const pristineFetch = window.fetch;

afterEach(() => {
  window.fetch = pristineFetch;
  vi.restoreAllMocks();
});

/** A minimal ok JSON Response factory. */
function jsonResponse(
  body: unknown,
  init?: { status?: number; contentType?: string },
): Response {
  const status = init?.status ?? 200;
  const headers = new Headers();
  if (init?.contentType !== null) {
    headers.set("content-type", init?.contentType ?? "application/json");
  }
  return new Response(JSON.stringify(body), { status, headers });
}

describe("installModeAFetchShim — request rewriting", () => {
  it("rewrites /api/v1/<path> to /api/proxy/api/v1/<path> and attaches x-wizard-csrf", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("/api/v1/keys");

    const [calledUrl, init] = base.mock.calls[0]!;
    expect(String(calledUrl)).toContain("/api/proxy/api/v1/keys");
    const headers = init.headers as Record<string, string>;
    expect(headers["x-wizard-csrf"]).toBe("csrf-token-xyz");
  });

  it("forwards the original method and body on rewritten /api/v1 requests", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("/api/v1/keys", {
      method: "POST",
      body: JSON.stringify({ slug: "x" }),
      headers: { "content-type": "application/json" },
    });

    const [, init] = base.mock.calls[0]!;
    expect(init.method).toBe("POST");
    expect(init.body).toBe(JSON.stringify({ slug: "x" }));
  });

  it("passes cross-origin requests through untouched", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("https://example.com/api/v1/keys");

    // Passed through as a Request object, not a rewritten string URL.
    const arg = base.mock.calls[0]![0] as Request;
    expect(arg).toBeInstanceOf(Request);
    expect(arg.url).toBe("https://example.com/api/v1/keys");
  });

  it("passes same-origin non-/api/v1 requests through untouched", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("/some/other/path");

    const arg = base.mock.calls[0]![0] as Request;
    expect(arg).toBeInstanceOf(Request);
    expect(new URL(arg.url).pathname).toBe("/some/other/path");
  });
});

describe("installModeAFetchShim — cli-pairings short-circuits", () => {
  it("returns a synthetic 200 {ok:true} for non-cancel cli-pairings calls without hitting the network", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    const res = await window.fetch("/api/v1/cli-pairings/pair-1/reserve-action", {
      method: "POST",
    });

    expect(base).not.toHaveBeenCalled();
    expect(res.status).toBe(200);
    await expect(res.json()).resolves.toEqual({ ok: true });
  });

  it("forwards a /cli-pairings/<id>/cancel to /api/proxy/cancel-unload with csrf", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("/api/v1/cli-pairings/pair-1/cancel", {
      method: "POST",
      keepalive: true,
    });

    expect(base).toHaveBeenCalledTimes(1);
    const [calledUrl, init] = base.mock.calls[0]!;
    expect(calledUrl).toBe("/api/proxy/cancel-unload");
    expect(init.method).toBe("POST");
    expect(init.keepalive).toBe(true);
    expect((init.headers as Record<string, string>)["x-wizard-csrf"]).toBe(
      "csrf-token-xyz",
    );
  });
});

describe("installModeAFetchShim — placeholder abandon", () => {
  it("rewrites DELETE /api/v1/keys/<id>?only_if_pending=true to POST /api/proxy/abandon-placeholder with {key_id}", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("/api/v1/keys/key-99?only_if_pending=true", {
      method: "DELETE",
    });

    expect(base).toHaveBeenCalledTimes(1);
    const [calledUrl, init] = base.mock.calls[0]!;
    expect(calledUrl).toBe("/api/proxy/abandon-placeholder");
    expect(init.method).toBe("POST");
    expect(init.body).toBe(JSON.stringify({ key_id: "key-99" }));
    expect((init.headers as Record<string, string>)["x-wizard-csrf"]).toBe(
      "csrf-token-xyz",
    );
  });

  it("does NOT short-circuit a DELETE /api/v1/keys/<id> without only_if_pending=true (rewrites as a normal /api/v1 call)", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await window.fetch("/api/v1/keys/key-99", { method: "DELETE" });

    const [calledUrl] = base.mock.calls[0]!;
    expect(String(calledUrl)).toContain("/api/proxy/api/v1/keys/key-99");
  });
});

describe("installModeAFetchShim — idempotent install", () => {
  it("replaces the bootstrap csrf on a second install but only wraps fetch once", async () => {
    const { installModeAFetchShim } = await freshClient();
    const base = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    window.fetch = base;

    installModeAFetchShim(BOOTSTRAP);
    const wrappedAfterFirst = window.fetch;
    installModeAFetchShim({ ...BOOTSTRAP, csrf: "second-csrf" });
    // Same wrapped function reference — fetch was not re-wrapped.
    expect(window.fetch).toBe(wrappedAfterFirst);

    await window.fetch("/api/v1/keys");
    const [, init] = base.mock.calls[0]!;
    // ...but the newer bootstrap's csrf is what gets attached.
    expect((init.headers as Record<string, string>)["x-wizard-csrf"]).toBe(
      "second-csrf",
    );
  });
});

describe("maybeDispatchUpstreamError / onUpstreamError", () => {
  it("emits a 'timeout' event for a non-ok JSON {error:'upstream_timeout'} response", async () => {
    const { installModeAFetchShim, onUpstreamError } = await freshClient();
    const base = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ error: "upstream_timeout" }, { status: 504 }),
      );
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    const seen: string[] = [];
    const unsub = onUpstreamError((kind) => seen.push(kind));

    await window.fetch("/api/v1/keys");
    // The clone().json() resolves on a microtask; flush it.
    await new Promise((r) => setTimeout(r, 0));

    expect(seen).toEqual(["timeout"]);
    unsub();
  });

  it("emits an 'unreachable' event for {error:'upstream_unreachable'}", async () => {
    const { installModeAFetchShim, onUpstreamError } = await freshClient();
    const base = vi
      .fn()
      .mockResolvedValue(
        jsonResponse({ error: "upstream_unreachable" }, { status: 502 }),
      );
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    const seen: string[] = [];
    const unsub = onUpstreamError((kind) => seen.push(kind));

    await window.fetch("/api/v1/keys");
    await new Promise((r) => setTimeout(r, 0));

    expect(seen).toEqual(["unreachable"]);
    unsub();
  });

  it("emits no event for an ok JSON response", async () => {
    const { installModeAFetchShim, onUpstreamError } = await freshClient();
    const base = vi
      .fn()
      .mockResolvedValue(jsonResponse({ error: "upstream_timeout" }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    const seen: string[] = [];
    const unsub = onUpstreamError((kind) => seen.push(kind));

    await window.fetch("/api/v1/keys");
    await new Promise((r) => setTimeout(r, 0));

    expect(seen).toEqual([]);
    unsub();
  });

  it("emits no event for a non-JSON non-ok response", async () => {
    const { installModeAFetchShim, onUpstreamError } = await freshClient();
    const base = vi.fn().mockResolvedValue(
      new Response("oops", {
        status: 500,
        headers: { "content-type": "text/plain" },
      }),
    );
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    const seen: string[] = [];
    const unsub = onUpstreamError((kind) => seen.push(kind));

    await window.fetch("/api/v1/keys");
    await new Promise((r) => setTimeout(r, 0));

    expect(seen).toEqual([]);
    unsub();
  });

  it("onUpstreamError unsubscribe removes the listener", async () => {
    const { onUpstreamError } = await freshClient();
    const seen: string[] = [];
    const unsub = onUpstreamError((kind) => seen.push(kind));
    unsub();

    window.dispatchEvent(
      new CustomEvent("nyxid-wizard-upstream-error", {
        detail: { kind: "timeout" },
      }),
    );

    expect(seen).toEqual([]);
  });
});

describe("postWizardComplete", () => {
  it("posts {body} to /api/proxy/complete with csrf and resolves on ok", async () => {
    const { installModeAFetchShim, postWizardComplete } = await freshClient();
    const base = vi.fn().mockResolvedValue(new Response(null, { status: 200 }));
    window.fetch = base;
    // Install so withCsrf has a bootstrap to read for the csrf header.
    // postWizardComplete calls the (now-wrapped) global fetch with a
    // string URL; /api/proxy/* is same-origin non-/api/v1 so the shim
    // forwards it as a Request to `base`. Read URL/method/csrf off that.
    installModeAFetchShim(BOOTSTRAP);

    await expect(
      postWizardComplete({ ack: "done" }),
    ).resolves.toBeUndefined();

    const req = base.mock.calls[0]![0] as Request;
    expect(new URL(req.url).pathname).toBe("/api/proxy/complete");
    expect(req.method).toBe("POST");
    expect(req.headers.get("x-wizard-csrf")).toBe("csrf-token-xyz");
    await expect(req.clone().text()).resolves.toBe(
      JSON.stringify({ ack: "done" }),
    );
  });

  it("throws on a non-ok response", async () => {
    const { installModeAFetchShim, postWizardComplete } = await freshClient();
    const base = vi.fn().mockResolvedValue(
      new Response(null, { status: 500, statusText: "Server Error" }),
    );
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await expect(postWizardComplete({})).rejects.toThrow(
      /\/api\/proxy\/complete failed: 500/,
    );
  });
});

describe("postWizardCancel", () => {
  it("posts to /api/proxy/cancel best-effort", async () => {
    const { installModeAFetchShim, postWizardCancel } = await freshClient();
    const base = vi.fn().mockResolvedValue(new Response(null, { status: 200 }));
    window.fetch = base;
    installModeAFetchShim(BOOTSTRAP);

    await postWizardCancel();

    const req = base.mock.calls[0]![0] as Request;
    expect(new URL(req.url).pathname).toBe("/api/proxy/cancel");
    expect(req.method).toBe("POST");
  });

  it("swallows a fetch rejection (never throws)", async () => {
    const { postWizardCancel } = await freshClient();
    window.fetch = vi.fn().mockRejectedValue(new Error("connection refused"));

    await expect(postWizardCancel()).resolves.toBeUndefined();
  });
});

describe("installHeartbeat", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("calls onDisconnect after 3 consecutive failed heartbeats and onReconnect on a later success", async () => {
    const { installHeartbeat } = await freshClient();

    const fetchMock = vi
      .fn()
      .mockRejectedValueOnce(new Error("refused"))
      .mockRejectedValueOnce(new Error("refused"))
      .mockRejectedValueOnce(new Error("refused"))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));
    window.fetch = fetchMock;

    const onDisconnect = vi.fn();
    const onReconnect = vi.fn();
    const cleanup = installHeartbeat({ onDisconnect, onReconnect });

    // Three failed ticks → disconnect. Each tick's .catch() runs on a
    // microtask, so advance timers AND flush promises after every tick.
    for (let i = 0; i < 3; i++) {
      await vi.advanceTimersByTimeAsync(1200);
    }
    expect(onDisconnect).toHaveBeenCalledTimes(1);
    expect(onReconnect).not.toHaveBeenCalled();

    // Fourth tick succeeds → reconnect.
    await vi.advanceTimersByTimeAsync(1200);
    expect(onReconnect).toHaveBeenCalledTimes(1);

    cleanup();
  });

  it("does not call onDisconnect before the 3rd consecutive failure", async () => {
    const { installHeartbeat } = await freshClient();
    window.fetch = vi.fn().mockRejectedValue(new Error("refused"));

    const onDisconnect = vi.fn();
    const cleanup = installHeartbeat({ onDisconnect });

    await vi.advanceTimersByTimeAsync(1200);
    await vi.advanceTimersByTimeAsync(1200);
    expect(onDisconnect).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1200);
    expect(onDisconnect).toHaveBeenCalledTimes(1);

    cleanup();
  });

  it("cleanup clears the interval so no further heartbeats fire", async () => {
    const { installHeartbeat } = await freshClient();
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 200 }));
    window.fetch = fetchMock;

    const cleanup = installHeartbeat({});
    await vi.advanceTimersByTimeAsync(1200);
    const callsAfterOneTick = fetchMock.mock.calls.length;
    expect(callsAfterOneTick).toBeGreaterThanOrEqual(1);

    cleanup();
    await vi.advanceTimersByTimeAsync(1200 * 5);
    // No additional heartbeats after cleanup.
    expect(fetchMock.mock.calls.length).toBe(callsAfterOneTick);
  });
});
