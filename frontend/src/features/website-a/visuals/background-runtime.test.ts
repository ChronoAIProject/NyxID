import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BackgroundRuntime } from "./background-runtime";

let load: () => Promise<BackgroundRuntime>;
beforeEach(async () => {
  vi.useFakeTimers();
  vi.resetModules();
  const append = document.head.appendChild.bind(document.head);
  vi.spyOn(document.head, "appendChild").mockImplementation(function <
    T extends Node,
  >(node: T): T {
    // Exercise loader events without Happy DOM executing or fetching the third-party script.
    if (node instanceof HTMLScriptElement) node.type = "application/json";
    return append(node);
  });
  ({ loadBackgroundRuntime: load } = await import("./background-runtime"));
});
afterEach(() => {
  document
    .querySelectorAll('script[src*="unicornstudio.js"]')
    .forEach((script) => script.remove());
  Reflect.deleteProperty(window, "UnicornStudio");
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("bounded shared hero runtime loader", () => {
  it("shares a single script load and resolves only when the runtime is present", async () => {
    const first = load();
    expect(load()).toBe(first);
    expect(
      document.querySelectorAll('script[src*="unicornstudio.js"]'),
    ).toHaveLength(1);
    const runtime: BackgroundRuntime = { addScene: vi.fn() };
    Object.defineProperty(window, "UnicornStudio", {
      value: runtime,
      configurable: true,
    });
    document
      .querySelector('script[src*="unicornstudio.js"]')!
      .dispatchEvent(new Event("load"));
    await expect(first).resolves.toBe(runtime);
    expect(vi.getTimerCount()).toBe(0);
    await expect(load()).resolves.toBe(runtime);
  });

  it("removes a failed script and allows a later route mount to retry", async () => {
    const failed = load();
    const check = expect(failed).rejects.toThrow("failed to load");
    const first = document.querySelector('script[src*="unicornstudio.js"]')!;
    first.dispatchEvent(new Event("error"));
    await check;
    expect(first.isConnected).toBe(false);
    const retried = load();
    const retryCheck = expect(retried).rejects.toThrow("failed to load");
    const second = document.querySelector('script[src*="unicornstudio.js"]')!;
    expect(second).not.toBe(first);
    second.dispatchEvent(new Event("error"));
    await retryCheck;
    expect(vi.getTimerCount()).toBe(0);
  });

  it("times out an unresponsive script without keeping timers or event handlers alive", async () => {
    const pending = load();
    const check = expect(pending).rejects.toThrow("timed out");
    const script = document.querySelector<HTMLScriptElement>(
      'script[src*="unicornstudio.js"]',
    )!;
    await vi.advanceTimersByTimeAsync(8000);
    await check;
    expect(script.isConnected).toBe(false);
    expect(script.onload).toBeNull();
    expect(script.onerror).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
  });
});
