// @vitest-environment jsdom
import { createElement } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NyxIDClient, type NyxIDTokenSet } from "@nyxids/oauth-core";
import { NyxIDProvider, useNyxID } from "../src/index.js";

afterEach(() => { cleanup(); vi.useRealTimers(); });

function fixture() {
  let stored: NyxIDTokenSet | null = { accessToken: "old", tokenType: "Bearer", expiresIn: 900 };
  const operations = {
    getStoredTokens: vi.fn(() => stored),
    getAccessToken: vi.fn(async () => stored?.accessToken ?? ""),
    refreshToken: vi.fn(async () => stored!),
    clearSession: vi.fn(() => { stored = null; }),
    getUserInfo: vi.fn(async () => ({ sub: "person" })),
  };
  let context!: ReturnType<typeof useNyxID>;
  function Consumer() { context = useNyxID(); return null; }
  const client = operations as unknown as NyxIDClient;
  const view = render(createElement(NyxIDProvider, { client, children: createElement(Consumer) }));
  return { operations, view, current: () => context, set: (tokens: NyxIDTokenSet | null) => { stored = tokens; } };
}

describe("React refresh lifecycle", () => {
  it("refreshes on mount and every 30 seconds and cleans up on unmount", async () => {
    vi.useFakeTimers();
    const f = fixture();
    await act(async () => {});
    expect(f.operations.getAccessToken).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(f.operations.getAccessToken).toHaveBeenCalledTimes(2);
    f.view.unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    expect(f.operations.getAccessToken).toHaveBeenCalledTimes(2);
  });

  it("synchronizes explicit refresh failure after invalid_grant clears tokens", async () => {
    const f = fixture();
    await act(async () => {});
    f.operations.refreshToken.mockImplementation(async () => { f.set(null); throw new Error("invalid_grant"); });
    await act(async () => { await expect(f.current().refreshToken()).rejects.toThrow("invalid_grant"); });
    expect(f.current().isAuthenticated).toBe(false);
  });

  it("retains authenticated context on retryable refresh failure", async () => {
    const f = fixture();
    await act(async () => {});
    f.operations.refreshToken.mockRejectedValue(new Error("temporarily_unavailable"));
    await act(async () => { await expect(f.current().refreshToken()).rejects.toThrow("temporarily_unavailable"); });
    expect(f.current().tokens?.accessToken).toBe("old");
  });

  it("observes tokens rotated or cleared in another tab", async () => {
    const f = fixture();
    await act(async () => {});
    f.set({ accessToken: "rotated", tokenType: "Bearer", expiresIn: 900 });
    await act(async () => { window.dispatchEvent(new Event("storage")); });
    expect(f.current().tokens?.accessToken).toBe("rotated");
    f.set(null);
    await act(async () => { window.dispatchEvent(new Event("storage")); });
    expect(f.current().isAuthenticated).toBe(false);
  });
});
