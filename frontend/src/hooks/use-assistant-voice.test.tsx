import type { VoiceConnectionStatus } from "@/lib/assistant-voice-client";
import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useAssistantVoice } from "./use-assistant-voice";
import type {
  VoicePreferences,
  VoiceSnapshot,
} from "@/schemas/assistant-voice";

const fixture = vi.hoisted(() => ({
  actor: "actor",
  status: undefined as undefined | ((value: VoiceConnectionStatus) => void),
  snapshot: undefined as undefined | ((value: VoiceSnapshot) => void),
  input: undefined as undefined | ((value: boolean) => void),
  closed: undefined as undefined | (() => void),
  conflict: undefined as undefined | ((value: unknown) => void),
  start: vi.fn(async () => {}),
  end: vi.fn(),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) =>
    select({ user: { id: fixture.actor } }),
}));
vi.mock("@/lib/assistant-voice-client", () => ({
  AssistantVoiceClient: class {
    isClosed = false;
    start = fixture.start; // POST resolves before the upstream relay is ready.
    end = fixture.end;
    hold = vi.fn();
    rejoin = vi.fn();
    constructor(
      _thread: string,
      snapshot: (value: VoiceSnapshot) => void,
      _error: unknown,
      _closed: unknown,
      _conflict: unknown,
      _activity: unknown,
      status: (value: VoiceConnectionStatus) => void,
      input: (enabled: boolean) => void,
    ) {
      fixture.snapshot = snapshot;
      fixture.status = status;
      fixture.input = input;
      fixture.closed = _closed as () => void;
      fixture.conflict = _conflict as (value: unknown) => void;
    }
  },
}));
afterEach(() => {
  vi.clearAllMocks();
  fixture.actor = "actor";
  localStorage.removeItem("nyxid:microphone-device:actor");
  localStorage.removeItem("nyxid:microphone-device:other-actor");
});
it.each(["openai_live", "xai_realtime"] as const)(
  "keeps the acting person's selected microphone on direct %s starts and retries",
  async (protocol) => {
    localStorage.setItem("nyxid:microphone-device:actor", "usb");
    localStorage.setItem("nyxid:microphone-device:other-actor", "built-in");
    const preferences = {} as VoicePreferences;
    const { result, rerender } = renderHook(() => useAssistantVoice("thread"));
    await act(() => result.current.start(preferences, protocol));
    expect(fixture.start).toHaveBeenLastCalledWith(
      preferences,
      protocol,
      undefined,
      undefined,
      "usb",
    );
    await act(() => result.current.end());
    localStorage.setItem("nyxid:microphone-device:actor", "replacement-usb");
    await act(() => result.current.start(preferences, protocol));
    expect(fixture.start).toHaveBeenLastCalledWith(
      preferences,
      protocol,
      undefined,
      undefined,
      "replacement-usb",
    );
    await act(() => result.current.end());
    fixture.actor = "other-actor";
    rerender();
    await act(() => result.current.start(preferences, protocol));
    expect(fixture.start).toHaveBeenLastCalledWith(
      preferences,
      protocol,
      undefined,
      undefined,
      "built-in",
    );
    await act(() => result.current.end());
  },
);
it.each(["openai_live", "xai_realtime"] as const)(
  "keeps %s connecting until the client reports media and control readiness",
  async (protocol) => {
    const { result } = renderHook(() => useAssistantVoice("thread"));
    await act(() => result.current.start({} as VoicePreferences, protocol));
    expect(result.current.starting).toBe(true);
    expect(result.current.connected).toBe(false);
    act(() =>
      fixture.snapshot?.({ session: { state: "starting" } } as VoiceSnapshot),
    );
    expect(result.current.starting).toBe(true);
    act(() =>
      fixture.snapshot?.({ session: { state: "active" } } as VoiceSnapshot),
    );
    expect(result.current.starting).toBe(true);
    expect(result.current.connected).toBe(false);
    act(() => fixture.status?.("ready"));
    expect(result.current.starting).toBe(false);
    expect(result.current.connected).toBe(true);
    await act(() => result.current.end());
    expect(fixture.end).toHaveBeenCalled();
    expect(result.current.connected).toBe(false);
  },
);
it("can end Grok before its provider connection is ready", async () => {
  const { result } = renderHook(() => useAssistantVoice("thread"));
  await act(() => result.current.start({} as VoicePreferences, "xai_realtime"));
  await act(() => result.current.end());
  expect(fixture.end).toHaveBeenCalled();
  expect(result.current.starting).toBe(false);
  act(() =>
    fixture.snapshot?.({ session: { state: "active" } } as VoiceSnapshot),
  );
  act(() => fixture.status?.("ready"));
  expect(result.current.connected).toBe(false); // Late snapshots cannot resurrect it.
});

it("disables talk and clears the held state while reconnecting", async () => {
  const { result } = renderHook(() => useAssistantVoice("thread"));
  await act(() => result.current.start({} as VoicePreferences));
  act(() => result.current.hold(true));
  expect(result.current.holding).toBe(false);
  act(() => fixture.status?.("ready"));
  act(() => result.current.hold(true));
  expect(result.current.holding).toBe(true);
  act(() => fixture.status?.("reconnecting"));
  expect(result.current.holding).toBe(false);
  expect(result.current.connected).toBe(false);
  expect(result.current.starting).toBe(true);
  expect(result.current.connectionStatus).toBe("reconnecting");
});

it("clears actual capture on end and fences callbacks from old threads and actors", async () => {
  const { result, rerender } = renderHook(
    ({ thread }) => useAssistantVoice(thread),
    { initialProps: { thread: "one" } },
  );
  await act(() => result.current.start({} as VoicePreferences));
  act(() => {
    fixture.status?.("ready");
    fixture.input?.(true);
  });
  expect(result.current.inputAccepting).toBe(true);
  const old = fixture.input!;
  await act(() => result.current.end());
  expect(result.current.inputAccepting).toBe(false);
  act(() => old(true));
  expect(result.current.inputAccepting).toBe(false);
  await act(() => result.current.start({} as VoicePreferences));
  act(() => {
    fixture.status?.("ready");
    fixture.input?.(true);
  });
  const previous = fixture.input!;
  rerender({ thread: "two" });
  expect(result.current.inputAccepting).toBe(false);
  act(() => previous(true));
  expect(result.current.inputAccepting).toBe(false);
  await act(() => result.current.start({} as VoicePreferences));
  act(() => {
    fixture.status?.("ready");
    fixture.input?.(true);
  });
  fixture.actor = "other";
  rerender({ thread: "two" });
  expect(result.current.inputAccepting).toBe(false);
});
it("clears capture during reconnect, closure, failed start and rejoin", async () => {
  const { result } = renderHook(() => useAssistantVoice("thread"));
  await act(() => result.current.start({} as VoicePreferences));
  act(() => {
    fixture.status?.("ready");
    fixture.input?.(true);
  });
  act(() => fixture.status?.("reconnecting"));
  expect(result.current.inputAccepting).toBe(false);
  act(() => {
    fixture.status?.("ready");
    fixture.input?.(true);
  });
  act(() => fixture.closed?.());
  expect(result.current.inputAccepting).toBe(false);
  act(() => fixture.conflict?.({ resumable: true }));
  act(() => result.current.rejoin());
  expect(result.current.inputAccepting).toBe(false);
  act(() => fixture.closed?.());
  expect(result.current.inputAccepting).toBe(false);
});
it("emits a restored connection only for a watched reconnect in the same session", async () => {
  vi.useFakeTimers();
  const { result } = renderHook(() => useAssistantVoice("thread"));
  await act(() => result.current.start({} as VoicePreferences));
  act(() => {
    fixture.snapshot?.({
      session: { id: "session", state: "active" },
    } as VoiceSnapshot);
    fixture.status?.("ready");
  });
  expect(result.current.restoredEvent).toBeNull();
  act(() => {
    fixture.status?.("reconnecting");
    fixture.status?.("ready");
    fixture.input?.(true);
  });
  expect(result.current.restoredEvent?.sessionId).toBe("session");
  expect(result.current.inputAccepting).toBe(true);
  const id = result.current.restoredEvent?.id;
  act(() => fixture.status?.("ready"));
  expect(result.current.restoredEvent?.id).toBe(id);
  act(() => vi.advanceTimersByTime(1300));
  expect(result.current.restoredEvent).toBeNull();
  vi.useRealTimers();
});
