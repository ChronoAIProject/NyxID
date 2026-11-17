import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createElement, useEffect, useState } from "react";
import { AccountBoundary } from "@/components/account-boundary";
import { useAuthStore } from "@/stores/auth-store";
import { installAccountSessionSync } from "./account-session-sync";
import type { User } from "@/types/api";

const person = (id: string) => ({ id, email: `${id}@example.test` }) as User;
let receive: (event: { data: unknown }) => void;
const post = vi.fn();
let stop: () => void;
beforeEach(() => {
  useAuthStore.getState().setUser(person("A"));
  useAuthStore.setState({ isIdentityVerifying: false, isLoading: false });
  post.mockReset();
  vi.stubGlobal(
    "BroadcastChannel",
    class {
      postMessage = post;
      close() {}
      set onmessage(fn: typeof receive) {
        receive = fn;
      }
    },
  );
  stop = installAccountSessionSync();
});
afterEach(() => {
  stop();
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

it.each(["broadcast", "storage", "bfcache"])(
  "hides A synchronously and verifies the cookie before publishing B via %s",
  async (trigger) => {
    let resolve!: (response: Response) => void;
    const fetch = vi.fn().mockReturnValue(
      new Promise<Response>((done) => {
        resolve = done;
      }),
    );
    vi.stubGlobal("fetch", fetch);
    function View() {
      const user = useAuthStore((state) => state.user);
      return createElement("p", {}, user?.id);
    }
    render(createElement(AccountBoundary, { children: createElement(View) }));
    expect(screen.getByText("A")).toBeVisible();
    act(() => {
      if (trigger === "broadcast") receive({ data: { userId: "B" } });
      if (trigger === "storage")
        window.dispatchEvent(
          new StorageEvent("storage", {
            key: "nyxid:account-session",
            newValue: JSON.stringify({ userId: "B" }),
          }),
        );
      if (trigger === "bfcache")
        window.dispatchEvent(
          Object.assign(new Event("pageshow"), { persisted: true }),
        );
      expect(useAuthStore.getState().isIdentityVerifying).toBe(true);
      expect(document.documentElement).toHaveAttribute(
        "data-account-verifying",
      );
    });
    expect(screen.getByText("A")).not.toBeVisible();
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    await act(async () => {
      resolve(Response.json(person("B")));
    });
    await vi.waitFor(() =>
      expect(useAuthStore.getState().isIdentityVerifying).toBe(false),
    );
    expect(screen.queryByText("A")).toBeNull();
    expect(screen.getByText("B")).toBeVisible();
    expect(post).not.toHaveBeenCalled();
  },
);

it("ignores malformed and same-account hints, and broadcasts confirmed changes only", async () => {
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  receive({ data: {} });
  receive({ data: { userId: 42 } });
  receive({ data: { userId: "A" } });
  useAuthStore.getState().setUser({ ...person("A"), display_name: "updated" });
  expect(fetch).not.toHaveBeenCalled();
  expect(post).not.toHaveBeenCalled();
  useAuthStore.getState().setUser(person("B"));
  expect(post).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ userId: "B" }),
  );
});

it("keeps same-account pages and drafts mounted through bfcache verification", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json(person("A"))));
  const unmount = vi.fn();
  function Draft() {
    const [text] = useState("draft survives");
    useEffect(() => unmount, []);
    return createElement("p", {}, text);
  }
  render(createElement(AccountBoundary, { children: createElement(Draft) }));
  await act(async () =>
    window.dispatchEvent(
      Object.assign(new Event("pageshow"), { persisted: true }),
    ),
  );
  await vi.waitFor(() =>
    expect(useAuthStore.getState().isIdentityVerifying).toBe(false),
  );
  expect(screen.getByText("draft survives")).toBeVisible();
  expect(unmount).not.toHaveBeenCalled();
});

it("retries transient verification errors without revealing the unconfirmed account", async () => {
  vi.useFakeTimers();
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(Response.json({}, { status: 503 }))
    .mockResolvedValueOnce(Response.json(person("B")));
  vi.stubGlobal("fetch", fetch);
  receive({ data: { userId: "B" } });
  await vi.advanceTimersByTimeAsync(0);
  expect(useAuthStore.getState().isIdentityVerifying).toBe(true);
  expect(document.documentElement).toHaveAttribute("data-account-verifying");
  await vi.advanceTimersByTimeAsync(5_000);
  expect(useAuthStore.getState().user?.id).toBe("B");
  expect(useAuthStore.getState().isIdentityVerifying).toBe(false);
});

it("only verifies stale visible pages and deduplicates in-flight checks", async () => {
  stop();
  vi.useFakeTimers();
  stop = installAccountSessionSync();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const fetch = vi.fn().mockResolvedValue(Response.json(person("A")));
  vi.stubGlobal("fetch", fetch);
  document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(59_999);
  expect(fetch).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  expect(fetch).toHaveBeenCalledOnce();
  expect(useAuthStore.getState().user?.id).toBe("A");
});
