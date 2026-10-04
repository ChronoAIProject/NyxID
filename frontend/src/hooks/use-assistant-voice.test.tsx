import { act, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useAssistantVoice } from "./use-assistant-voice";
import type {
  VoicePreferences,
  VoiceSnapshot,
} from "@/schemas/assistant-voice";

const fixture = vi.hoisted(() => ({
  snapshot: undefined as undefined | ((value: VoiceSnapshot) => void),
  end: vi.fn(),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) =>
    select({ user: { id: "actor" } }),
}));
vi.mock("@/lib/assistant-voice-client", () => ({
  AssistantVoiceClient: class {
    isClosed = false;
    start = vi.fn(async () => {}); // POST resolves before the upstream relay is ready.
    end = fixture.end;
    constructor(_thread: string, snapshot: (value: VoiceSnapshot) => void) {
      fixture.snapshot = snapshot;
    }
  },
}));
afterEach(() => {
  vi.clearAllMocks();
});
it("keeps Grok connecting and cancellable until the server reports an active relay", async () => {
  const { result } = renderHook(() => useAssistantVoice("thread"));
  await act(() => result.current.start({} as VoicePreferences, "xai_realtime"));
  expect(result.current.starting).toBe(true);
  expect(result.current.connected).toBe(false);
  act(() =>
    fixture.snapshot?.({ session: { state: "starting" } } as VoiceSnapshot),
  );
  expect(result.current.starting).toBe(true);
  act(() =>
    fixture.snapshot?.({ session: { state: "active" } } as VoiceSnapshot),
  );
  expect(result.current.starting).toBe(false);
  expect(result.current.connected).toBe(true);
  await act(() => result.current.end());
  expect(fixture.end).toHaveBeenCalled();
  expect(result.current.connected).toBe(false);
});
it("can end Grok before its provider connection is ready", async () => {
  const { result } = renderHook(() => useAssistantVoice("thread"));
  await act(() => result.current.start({} as VoicePreferences, "xai_realtime"));
  await act(() => result.current.end());
  expect(fixture.end).toHaveBeenCalled();
  expect(result.current.starting).toBe(false);
  act(() =>
    fixture.snapshot?.({ session: { state: "active" } } as VoiceSnapshot),
  );
  expect(result.current.connected).toBe(false); // Late snapshots cannot resurrect it.
});
