import { act, renderHook } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ conversations: vi.fn(), history: vi.fn(), start: vi.fn(), subscribe: vi.fn(), apply: vi.fn(), stop: vi.fn() }));
vi.mock("@/router", () => ({ router: { subscribe: mocks.subscribe } }));
vi.mock("@/lib/build-updates", () => ({ startBuildUpdates: mocks.start }));
vi.mock("@/lib/assistant/direct-transport", () => ({ directAssistantTransport: { getConversationsSnapshot: mocks.conversations, getHistorySnapshot: mocks.history } }));
import { useBuildUpdateStore, UPDATE_REMINDER_DELAY } from "@/stores/build-update-store";
import { BUILD_UPDATE_VIEW_SWITCH } from "@/lib/build-update-navigation";
import { useBuildUpdates } from "./use-build-updates";

let hidden = false;
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubEnv("PROD", true);
  vi.clearAllMocks();
  useBuildUpdateStore.setState({ pending: null });
  mocks.conversations.mockReturnValue([]);
  hidden = false;
  window.history.replaceState({}, "", "/dashboard");
  document.body.innerHTML = "";
  document.querySelector('meta[name="nyxid-assistant-build-id"]')?.remove();
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  mocks.subscribe.mockReturnValue(vi.fn());
  mocks.start.mockReturnValue({ apply: mocks.apply, stop: mocks.stop });
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
});

function mount() {
  const client = new QueryClient();
  const hook = renderHook(() => useBuildUpdates(true, client));
  const gate = mocks.start.mock.calls[0]![0].canAutoReload as () => boolean;
  function hide() {
    hidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
    vi.advanceTimersByTime(60_000);
  }
  return { ...hook, client, gate, hide };
}

describe("automatic update safety", () => {
  it("only allows an untouched reading tab hidden for at least a minute", () => {
    const { gate, hide, unmount } = mount();
    expect(gate()).toBe(false);
    hide();
    expect(gate()).toBe(true);
    hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(gate()).toBe(false);
    unmount();
    expect(mocks.stop).toHaveBeenCalledOnce();
  });

  it.each(["pointerdown", "keydown", "input", "change", "submit", "drop"])("keeps interacted tabs intact after %s", (event) => {
    const { gate, hide, unmount } = mount();
    document.dispatchEvent(new Event(event));
    hide();
    expect(gate()).toBe(false);
    unmount();
  });

  it.each(["/assistant", "/login/device", "/cli/pair", "/keys/api-key/new", "/ssh/service/terminal", "/dashboard?claim=bearer"])("never automatically refreshes a stateful or handoff route: %s", (route) => {
    window.history.replaceState({}, "", route);
    const { gate, hide, unmount } = mount();
    hide();
    expect(gate()).toBe(false);
    unmount();
  });

  it("blocks after SPA navigation even when the user returns to a reading page", () => {
    const { gate, hide, unmount } = mount();
    mocks.subscribe.mock.calls[0]![1]();
    hide();
    expect(gate()).toBe(false);
    unmount();
  });

  it.each(['<input value="draft">', '<textarea>draft</textarea>', '<div contenteditable="true">draft</div>', '<div role="dialog">One-time secret</div>'])("blocks ephemeral controls and dialogs: %s", (html) => {
    const { gate, hide, unmount } = mount();
    document.body.insertAdjacentHTML("beforeend", html);
    hide();
    expect(gate()).toBe(false);
    unmount();
  });

  it("blocks pending mutations and only updates the hook when a build becomes ready", () => {
    const { client, gate, hide, result, unmount } = mount();
    vi.spyOn(client, "isMutating").mockReturnValue(1);
    hide();
    expect(gate()).toBe(false);
    expect(result.current.available).toBe(false);
    act(() => mocks.start.mock.calls[0]![0].onReady({ buildId: "next", commit: null, assets: ["assets/next.js"] }));
    expect(result.current.available).toBe(false);
    act(() => vi.advanceTimersByTime(UPDATE_REMINDER_DELAY));
    expect(result.current.available).toBe(true);
    unmount();
  });

  it("does not poll development builds", () => {
    vi.stubEnv("PROD", false);
    const { unmount } = renderHook(() => useBuildUpdates(true, new QueryClient()));
    expect(mocks.start).not.toHaveBeenCalled();
    unmount();
  });

  it("keeps the original reminder deadline through repeated confirmations", () => {
    const { result, unmount } = mount();
    const build = { buildId: "next", commit: null, assets: ["assets/next.js"] };
    act(() => mocks.start.mock.calls[0]![0].onReady(build));
    act(() => vi.advanceTimersByTime(UPDATE_REMINDER_DELAY - 1));
    act(() => mocks.start.mock.calls[0]![0].onReady(build));
    expect(result.current.available).toBe(false);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current.available).toBe(true);
    unmount();
  });

  it("applies a pending update only after an explicit view switch", () => {
    window.history.replaceState({}, "", "/assistant?c=old");
    const { unmount } = mount();
    act(() => mocks.start.mock.calls[0]![0].onReady({ buildId: "next", commit: null, assets: ["assets/next.js"] }));
    mocks.subscribe.mock.calls[1]![1]();
    expect(mocks.apply).not.toHaveBeenCalled();
    window.history.replaceState({}, "", "/assistant?c=new");
    window.dispatchEvent(new Event(BUILD_UPDATE_VIEW_SWITCH));
    expect(mocks.apply).toHaveBeenCalledOnce();
    const gate = mocks.apply.mock.calls[0]![0];
    expect(gate()).toBe(true);
    document.body.innerHTML = '<textarea>new draft</textarea>';
    expect(gate()).toBe(false);
    unmount();
    document.body.innerHTML = "";
    window.dispatchEvent(new Event(BUILD_UPDATE_VIEW_SWITCH));
    expect(mocks.apply).toHaveBeenCalledOnce();
  });

  it.each(["running", "waiting"])("defers view updates while a local turn is %s", (status) => {
    const { unmount } = mount();
    act(() => mocks.start.mock.calls[0]![0].onReady({ buildId: "next", commit: null, assets: ["assets/next.js"] }));
    mocks.conversations.mockReturnValue([{ id: "direct" }]);
    mocks.history.mockReturnValue({ activeTurn: { status } });
    window.dispatchEvent(new Event(BUILD_UPDATE_VIEW_SWITCH));
    expect(mocks.apply).not.toHaveBeenCalled();
    unmount();
  });

  it("clears a pending update when observation confirms the loaded build", () => {
    const { unmount } = mount();
    act(() => mocks.start.mock.calls[0]![0].onReady({ buildId: "next", commit: null, assets: ["assets/next.js"] }));
    act(() => mocks.start.mock.calls[0]![0].onObserved({ buildId: __BUILD_ID__ }));
    expect(useBuildUpdateStore.getState().pending).toBeNull();
    unmount();
  });

  it("does not activate an unrelated build on a chat switch", () => {
    const fingerprint = "a".repeat(64);
    document.head.insertAdjacentHTML("beforeend", `<meta name="nyxid-assistant-build-id" content="${fingerprint}">`);
    const { unmount } = mount();
    act(() => mocks.start.mock.calls[0]![0].onReady({ buildId: "next", commit: null, assistant: fingerprint, assets: ["assets/next.js"] }));
    window.dispatchEvent(new Event(BUILD_UPDATE_VIEW_SWITCH));
    expect(mocks.apply).not.toHaveBeenCalled();
    unmount();
  });

  it("activates on a resolved user link but defers while saves are pending", () => {
    const { client, unmount } = mount();
    act(() => mocks.start.mock.calls[0]![0].onReady({ buildId: "next", commit: null, assets: ["assets/next.js"] }));
    const anchor = document.createElement("a");
    anchor.href = "/assistant";
    document.body.append(anchor);
    anchor.addEventListener("click", (event) => event.preventDefault());
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    window.history.replaceState({}, "", "/assistant");
    mocks.subscribe.mock.calls[1]![1]();
    expect(mocks.apply).toHaveBeenCalledOnce();
    mocks.apply.mockClear();
    vi.spyOn(client, "isMutating").mockReturnValue(1);
    window.dispatchEvent(new Event(BUILD_UPDATE_VIEW_SWITCH));
    expect(mocks.apply).not.toHaveBeenCalled();
    unmount();
  });
});
