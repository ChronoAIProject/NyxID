import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  isAssetLoadError,
  recoverFromAssetError,
  retryAfterAssetError,
} from "@/lib/chunk-recovery";

const KEY = "nyxid_chunk_reload";

beforeEach(() => {
  // Restore the real storage object first: a stub installed by the previous
  // test is still in place until it is unstubbed.
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  window.sessionStorage.clear();
});

/**
 * Swap `window.sessionStorage` wholesale for a stub whose named method throws,
 * modelling Safari private browsing / sandboxed iframes / storage disabled.
 * Replacing the object is necessary because happy-dom implements Storage as a
 * Proxy: spying the instance is routed into the stored data, and spying
 * `Storage.prototype` is bypassed by the proxy's own handler.
 */
function withThrowingStorage(method: "getItem" | "setItem" | "removeItem") {
  const real = window.sessionStorage;
  const stub = {
    getItem: (k: string) => real.getItem(k),
    setItem: (k: string, v: string) => real.setItem(k, v),
    removeItem: (k: string) => real.removeItem(k),
    clear: () => real.clear(),
    [method]: () => {
      throw new Error("storage disabled");
    },
  };
  vi.stubGlobal("sessionStorage", stub);
}

describe("isAssetLoadError", () => {
  it.each([
    // Chrome/Edge
    "Failed to fetch dynamically imported module: https://x/assets/keys-A1.js",
    // Firefox — differs in wording *and* case, which the previous matcher missed
    "error loading dynamically imported module: https://x/assets/keys-A1.js",
    // Safari
    "Importing a module script failed.",
    // Missing chunk served as an HTML error page
    "Failed to load module script: Expected a JavaScript module script but the server responded with a MIME type of text/html.",
    // Vite's CSS preload rejection
    "Unable to preload CSS for /assets/keys-A1.css",
    "Loading chunk 42 failed.",
  ])("recognises %s", (message) => {
    expect(isAssetLoadError(new Error(message))).toBe(true);
  });

  it("recognises a ChunkLoadError by name even with an empty message", () => {
    const error = new Error("");
    error.name = "ChunkLoadError";
    expect(isAssetLoadError(error)).toBe(true);
  });

  it("does not claim genuine render errors", () => {
    expect(
      isAssetLoadError(new TypeError("Cannot read properties of undefined")),
    ).toBe(false);
  });

  it("tolerates non-Error throwables", () => {
    expect(isAssetLoadError("Failed to fetch dynamically imported module")).toBe(
      true,
    );
    expect(isAssetLoadError({ message: "Importing a module script failed." })).toBe(
      true,
    );
    expect(isAssetLoadError(undefined)).toBe(false);
    expect(isAssetLoadError(null)).toBe(false);
    expect(isAssetLoadError(42)).toBe(false);
  });
});

describe("recoverFromAssetError", () => {
  it("reloads once and records the build it reloaded for", () => {
    const reload = vi.fn();

    expect(recoverFromAssetError({ buildId: "build-1", reload })).toBe(
      "reloading",
    );

    expect(reload).toHaveBeenCalledTimes(1);
    expect(window.sessionStorage.getItem(KEY)).toBe("build-1");
  });

  it("refuses a second reload on the same build", () => {
    const reload = vi.fn();

    recoverFromAssetError({ buildId: "build-1", reload });
    expect(recoverFromAssetError({ buildId: "build-1", reload })).toBe(
      "exhausted",
    );

    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("is idempotent across call sites observing the same failure", () => {
    const reload = vi.fn();

    // `vite:preloadError` and the route error component can both fire.
    const first = recoverFromAssetError({ buildId: "build-1", reload });
    const second = recoverFromAssetError({ buildId: "build-1", reload });

    expect([first, second]).toEqual(["reloading", "exhausted"]);
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("does not loop when the guard survives the reload", () => {
    // Regression test for #357's guard, which was cleared on every bootstrap.
    // Bootstrap runs from the entry chunk, which loads fine; it is the route
    // chunk that fails — so the guard was wiped before the failure it gated and
    // a permanently missing chunk reloaded forever. Storage persists here, as
    // sessionStorage does across a real reload, so the second attempt must stop.
    const reload = vi.fn();

    for (let i = 0; i < 5; i += 1) {
      recoverFromAssetError({ buildId: "build-1", reload });
    }

    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("re-arms on a new build so the next deploy still auto-reloads", () => {
    const reload = vi.fn();

    recoverFromAssetError({ buildId: "build-1", reload });
    expect(recoverFromAssetError({ buildId: "build-2", reload })).toBe(
      "reloading",
    );

    expect(reload).toHaveBeenCalledTimes(2);
    expect(window.sessionStorage.getItem(KEY)).toBe("build-2");
  });

  it("declines to reload when the guard cannot be persisted", () => {
    // Without durable storage the reload could not be bounded, and an unbounded
    // reload is worse for the user than an error message.
    withThrowingStorage("setItem");
    const reload = vi.fn();

    expect(recoverFromAssetError({ buildId: "build-1", reload })).toBe(
      "exhausted",
    );
    expect(reload).not.toHaveBeenCalled();
  });

  it("survives a throwing storage read without escaping", () => {
    // A throw here used to take down the whole tree: the old boundary read
    // storage unguarded inside `getDerivedStateFromError`.
    withThrowingStorage("getItem");
    const reload = vi.fn();

    expect(() =>
      recoverFromAssetError({ buildId: "build-1", reload }),
    ).not.toThrow();
  });
});

describe("retryAfterAssetError", () => {
  it("clears the guard so an explicit retry is not capped", () => {
    const reload = vi.fn();
    recoverFromAssetError({ buildId: "build-1", reload });

    retryAfterAssetError(reload);

    expect(reload).toHaveBeenCalledTimes(2);
    expect(window.sessionStorage.getItem(KEY)).toBeNull();
    // And the automatic path is armed again for this build.
    expect(recoverFromAssetError({ buildId: "build-1", reload })).toBe(
      "reloading",
    );
  });
});
