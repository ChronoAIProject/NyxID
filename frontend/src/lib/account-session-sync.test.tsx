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
let style: HTMLStyleElement;
beforeEach(() => {
  useAuthStore.getState().setUser(person("A"));
  useAuthStore.setState({ isIdentityVerifying: false, isLoading: false });
  post.mockReset();
  style = document.createElement("style");
  style.textContent = "html[data-account-verifying] body { visibility: hidden; }";
  document.head.append(style);
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
  style.remove();
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
    expect(post).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ userId: "B" }),
    );
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

it.each([null, "B"])("broadcasts local identity %s while background verification is pending", async (id) => {
  stop();
  vi.useFakeTimers();
  stop = installAccountSessionSync();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  let resolve!: (response: Response) => void;
  const fetch = vi.fn((url: string) => String(url).endsWith("/auth/logout")
    ? Promise.resolve(new Response(null, { status: 204 }))
    : new Promise<Response>((done) => { resolve = done; }));
  vi.stubGlobal("fetch", fetch);
  await vi.advanceTimersByTimeAsync(60_000);
  expect(fetch).toHaveBeenCalledOnce();
  expect(post).not.toHaveBeenCalled();

  if (id === null) await useAuthStore.getState().logout();
  else useAuthStore.getState().setUser(person(id));
  expect(post).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ userId: id }));
  expect(JSON.parse(localStorage.getItem("nyxid:account-session")!)).toEqual(post.mock.calls[0]![0]);

  // The held old-account response must not undo the newer local confirmation.
  resolve(Response.json(person("A")));
  await vi.advanceTimersByTimeAsync(0);
  expect(useAuthStore.getState().user?.id ?? null).toBe(id);
  expect(post).toHaveBeenCalledOnce();
});

it("rebroadcasts a remote confirmation and ignores its echo at the originating identity", async () => {
  const fetch = vi.fn().mockResolvedValue(Response.json(person("B")));
  vi.stubGlobal("fetch", fetch);
  receive({ data: { userId: "B", nonce: "origin-confirmed-B" } });
  await vi.waitFor(() => expect(useAuthStore.getState().isIdentityVerifying).toBe(false));
  expect(useAuthStore.getState().user?.id).toBe("B");
  expect(post).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ userId: "B" }));
  const echo = post.mock.calls[0]![0];

  // The originating tab is already B. Both delivery mechanisms ignore this
  // receiver's new nonce because its confirmed target matches that identity.
  receive({ data: echo });
  window.dispatchEvent(new StorageEvent("storage", {
    key: "nyxid:account-session", newValue: JSON.stringify(echo),
  }));
  await Promise.resolve();
  expect(fetch).toHaveBeenCalledOnce();
  expect(post).toHaveBeenCalledOnce();
  expect(useAuthStore.getState().isIdentityVerifying).toBe(false);
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

it.each([401, 503, "network"])("fails closed and reveals the signed-out surface on evidence verification failure %s", async (status) => {
  vi.stubGlobal("fetch", vi.fn(async () => {
    if (status === "network") throw new TypeError("Offline");
    return Response.json({}, { status: Number(status) });
  }));
  receive({ data: { userId: "B" } });
  await vi.waitFor(() => expect(useAuthStore.getState().isIdentityVerifying).toBe(false));
  expect(useAuthStore.getState().user).toBeNull();
  expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
});

it("bounds an unresponsive evidence check to five seconds", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("fetch", vi.fn(() => new Promise(() => {})));
  receive({ data: { userId: "B" } });
  await vi.advanceTimersByTimeAsync(5_000);
  expect(useAuthStore.getState().user).toBeNull();
  expect(useAuthStore.getState().isIdentityVerifying).toBe(false);
  expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
});

it("deduplicates channel/storage hints by nonce and nonce-less duplicate target", async () => {
  let resolve!: (response: Response) => void;
  const fetch = vi.fn(() => new Promise<Response>((done) => { resolve = done; }));
  vi.stubGlobal("fetch", fetch);
  receive({ data: { userId: "B", nonce: "one" } });
  window.dispatchEvent(new StorageEvent("storage", { key: "nyxid:account-session", newValue: JSON.stringify({ userId: "B", nonce: "one" }) }));
  receive({ data: { userId: "B" } });
  await vi.waitFor(() => expect(fetch).toHaveBeenCalledOnce());
  resolve(Response.json(person("B")));
  await vi.waitFor(() => expect(useAuthStore.getState().isIdentityVerifying).toBe(false));
  expect(fetch).toHaveBeenCalledOnce();
});

it("never polls anonymous pages", async () => {
  useAuthStore.getState().setUser(null);
  vi.useFakeTimers();
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  await vi.advanceTimersByTimeAsync(180_000);
  document.dispatchEvent(new Event("visibilitychange"));
  expect(fetch).not.toHaveBeenCalled();
  expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
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

it("keeps background checks silent, retaining identity, loading, mount, focus and concurrent requests", async () => {
  stop();
  vi.useFakeTimers();
  stop = installAccountSessionSync();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  let resolve!: (response: Response) => void;
  const user = useAuthStore.getState().user;
  const fetch = vi.fn((url: string) => String(url).endsWith("/users/me")
    ? new Promise<Response>((done) => { resolve = done; }) : Promise.resolve(Response.json({ ok: true })));
  vi.stubGlobal("fetch", fetch);
  const unmount = vi.fn();
  function Draft() { useEffect(() => unmount, []); return createElement("input", { defaultValue: "private draft", "aria-label": "Draft" }); }
  render(createElement(AccountBoundary, { children: createElement(Draft) }));
  screen.getByRole("textbox").focus();
  for (let minute = 0; minute < 3; minute++) {
    await act(() => vi.advanceTimersByTimeAsync(60_000));
    expect(useAuthStore.getState().isLoading).toBe(false);
    expect(useAuthStore.getState().isIdentityVerifying).toBe(false);
    expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
    const { apiClient } = await import("./api-client");
    await expect(apiClient("/concurrent")).resolves.toEqual({ ok: true });
    await act(async () => resolve(Response.json(user)));
    expect(useAuthStore.getState().user).toBe(user);
    expect(screen.getByRole("textbox")).toHaveFocus();
    expect(screen.getByRole("textbox")).toHaveValue("private draft");
  }
  expect(unmount).not.toHaveBeenCalled();
  expect(fetch).toHaveBeenCalledTimes(6);
});

it.each([401, 503, "network"])("background failure %s never hides or toggles loading", async (status) => {
  stop();
  vi.useFakeTimers();
  stop = installAccountSessionSync();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const user = useAuthStore.getState().user;
  vi.stubGlobal("fetch", vi.fn(async () => {
    if (status === "network") throw new TypeError("Offline");
    return Response.json({}, { status: Number(status) });
  }));
  await vi.advanceTimersByTimeAsync(60_000);
  expect(useAuthStore.getState().user).toBe(status === 401 ? null : user);
  expect(useAuthStore.getState().isLoading).toBe(false);
  expect(useAuthStore.getState().isIdentityVerifying).toBe(false);
  expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
});

it("times out a hung background check and retries at the next interval without losing the account", async () => {
  stop();
  vi.useFakeTimers();
  stop = installAccountSessionSync();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const user = useAuthStore.getState().user;
  const fetch = vi.fn().mockImplementationOnce(() => new Promise(() => {})).mockResolvedValue(Response.json(user));
  vi.stubGlobal("fetch", fetch);
  await vi.advanceTimersByTimeAsync(75_000);
  expect(useAuthStore.getState().user).toBe(user);
  await vi.advanceTimersByTimeAsync(45_000);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(useAuthStore.getState().user).toBe(user);
});

it.each(["late failure", "deadline"])("does not clear a newer locally confirmed account on an obsolete evidence %s", async (scenario) => {
  let fail!: (error: Error) => void;
  vi.stubGlobal("fetch", vi.fn(() => new Promise<Response>((_, reject) => { fail = reject; })));
  receive({ data: { userId: "B" } });
  await vi.waitFor(() => expect(fail).toBeDefined());
  useAuthStore.getState().setUser(person("C"));
  if (scenario === "deadline") {
    vi.useFakeTimers();
    // Restart with fake timers before installing the attempt's deadline.
    stop();
    stop = installAccountSessionSync();
    receive({ data: { userId: "B" } });
    useAuthStore.getState().setUser(person("D"));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(useAuthStore.getState().user?.id).toBe("D");
  } else {
    fail(new TypeError("Old network failure"));
    await vi.waitFor(() => expect(useAuthStore.getState().isIdentityVerifying).toBe(false));
    expect(useAuthStore.getState().user?.id).toBe("C");
  }
  expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
});

it("does not poll a hidden signed-in tab, then silently checks on stale visibility return", async () => {
  stop();
  vi.useFakeTimers();
  stop = installAccountSessionSync();
  const visible = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  const fetch = vi.fn().mockResolvedValue(Response.json(person("A")));
  vi.stubGlobal("fetch", fetch);
  await vi.advanceTimersByTimeAsync(180_000);
  expect(fetch).not.toHaveBeenCalled();
  visible.mockReturnValue("visible");
  document.dispatchEvent(new Event("visibilitychange"));
  await vi.advanceTimersByTimeAsync(0);
  expect(fetch).toHaveBeenCalledOnce();
  expect(document.documentElement).not.toHaveAttribute("data-account-verifying");
  expect(useAuthStore.getState().isLoading).toBe(false);
});
