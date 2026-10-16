import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { AssistantDictationClient } from "./assistant-dictation-client";
import { voicePreferences } from "@/test/voice-fixture";
const f = vi.hoisted(() => ({ pcm: vi.fn(), close: vi.fn(), start: vi.fn() }));
vi.mock("./dictation-pcm", () => ({
  DictationPcm: class {
    start = f.start;
    close = f.close;
  },
}));
vi.mock("./microphone-capture", async (original) => ({
  ...(await original<typeof import("./microphone-capture")>()),
  AudioActivityMeter: class {
    watch = vi.fn();
    close = vi.fn();
  },
}));
class Socket {
  static OPEN = 1;
  static current: Socket;
  readyState = 1;
  bufferedAmount = 0;
  onopen?: () => void;
  onclose?: () => void;
  onerror?: () => void;
  onmessage?: ({ data }: { data: string }) => void;
  send = vi.fn();
  close = vi.fn();
  constructor() {
    Socket.current = this;
  }
  event(value: unknown) {
    this.onmessage?.({ data: JSON.stringify(value) });
  }
}
const sid = "22222222-2222-4222-8222-222222222222";
let stop: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  f.start.mockImplementation(async (_stream: MediaStream, pcm: unknown) => {
    f.pcm(pcm);
  });
  stop = vi.fn();
  vi.stubGlobal("WebSocket", Socket);
  vi.stubGlobal("navigator", {
    mediaDevices: {
      getUserMedia: vi.fn().mockResolvedValue({
        getTracks: () => [{ stop }],
        getAudioTracks: () => [{ stop }],
      }),
    },
  });
});
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
function fixture() {
  const callbacks = {
    text: vi.fn(),
    activity: vi.fn(),
    state: vi.fn(),
    error: vi.fn(),
  };
  return { callbacks, client: new AssistantDictationClient(callbacks) };
}
it("orders completions by canonical item order, replaces deltas and deduplicates finalized items", async () => {
  const { client, callbacks } = fixture();
  await client.start("thread", voicePreferences);
  const ws = Socket.current;
  ws.onopen?.();
  expect(JSON.parse(ws.send.mock.calls[0]![0] as string)).toMatchObject({
    preferences: voicePreferences,
  });
  ws.event({ type: "ready", model: "gpt-live-transcribe", session_id: sid });
  ws.event({ type: "delta", item_id: "B", order: 1, text: "second interim" });
  ws.event({ type: "completed", item_id: "A", order: 0, text: "first" });
  expect(callbacks.text).toHaveBeenLastCalledWith({
    final: "first",
    interim: "second interim",
    parts: [
      { text: "first", complete: true, order: 0 },
      { text: "second interim", complete: false, order: 1 },
    ],
  });
  ws.event({ type: "completed", item_id: "B", order: 1, text: "second" });
  ws.event({ type: "delta", item_id: "A", order: 0, text: "late" });
  ws.event({ type: "completed", item_id: "A", order: 0, text: "duplicate" });
  expect(callbacks.text.mock.lastCall![0].final).toBe("first second");
  client.stop();
  expect(stop).toHaveBeenCalledOnce();
  expect(ws.send).toHaveBeenLastCalledWith('{"type":"stop"}');
  ws.event({ type: "stopped", confirmed: true });
  expect(callbacks.state).toHaveBeenLastCalledWith("review");
  const count = callbacks.text.mock.calls.length;
  ws.event({ type: "delta", item_id: "late", order: 2, text: "no" });
  expect(callbacks.text).toHaveBeenCalledTimes(count);
});
it("preserves live display ordering when the later item completes first", async () => {
  const { client, callbacks } = fixture();
  await client.start("thread", voicePreferences);
  const ws = Socket.current;
  ws.event({ type: "delta", item_id: "A", order: 0, text: "earlier" });
  ws.event({ type: "completed", item_id: "B", order: 1, text: "later" });
  expect(
    callbacks.text.mock.lastCall![0].parts.map((p: { text: string }) => p.text),
  ).toEqual(["earlier", "later"]);
  client.cancel();
  ws.event({ type: "stopped", confirmed: true });
});
it("cancel while permission pending never opens a provider relay and stops a late grant", async () => {
  let grant!: (value: unknown) => void;
  vi.stubGlobal("navigator", {
    mediaDevices: {
      getUserMedia: () =>
        new Promise((resolve) => {
          grant = resolve;
        }),
    },
  });
  const { client, callbacks } = fixture();
  const started = client.start("thread", voicePreferences);
  client.cancel();
  grant({ getTracks: () => [{ stop }], getAudioTracks: () => [{ stop }] });
  await started;
  expect(stop).toHaveBeenCalledOnce();
  expect(callbacks.state).toHaveBeenLastCalledWith("review");
  expect(callbacks.error).not.toHaveBeenCalled();
});
it("denied microphone permission creates no websocket", async () => {
  const ws = Socket.current;
  vi.stubGlobal("navigator", {
    mediaDevices: {
      getUserMedia: vi
        .fn()
        .mockRejectedValue(new DOMException("private", "NotAllowedError")),
    },
  });
  const { client, callbacks } = fixture();
  await client.start("thread", voicePreferences);
  expect(Socket.current).toBe(ws);
  expect(callbacks.error).toHaveBeenCalledWith(
    expect.stringContaining("denied"),
  );
});

it("gesture capture precedes async thread/source preparation; cancellation during preparation never opens a socket", async () => {
  const { client, callbacks } = fixture();
  let resolve!: (value: {
    thread: string;
    preferences: typeof voicePreferences;
  }) => void;
  const prepare = vi.fn(
    () =>
      new Promise<{ thread: string; preferences: typeof voicePreferences }>(
        (done) => {
          resolve = done;
        },
      ),
  );
  const sockets = vi.fn();
  class DeferredSocket extends Socket {
    constructor() {
      super();
      sockets();
    }
  }
  vi.stubGlobal("WebSocket", DeferredSocket);
  const pending = client.start(prepare, undefined, "usb");
  expect(navigator.mediaDevices.getUserMedia).toHaveBeenCalledWith(
    expect.objectContaining({
      audio: expect.objectContaining({ deviceId: { exact: "usb" } }),
    }),
  );
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  expect(prepare).toHaveBeenCalledOnce();
  expect(sockets).not.toHaveBeenCalled();
  client.stop();
  resolve({ thread: "adopted", preferences: voicePreferences });
  await pending;
  expect(sockets).not.toHaveBeenCalled();
  expect(stop).toHaveBeenCalledOnce();
  expect(callbacks.state).toHaveBeenLastCalledWith("review");
});

it("Stop during PCM startup ignores its late rejection and drains the final transcript before review", async () => {
  let rejectEncoder!: (error: unknown) => void;
  f.start.mockImplementationOnce(
    () =>
      new Promise((_resolve, reject) => {
        rejectEncoder = reject;
      }),
  );
  const { client, callbacks } = fixture();
  await client.start("thread", voicePreferences);
  const ws = Socket.current;
  ws.event({ type: "ready", model: "gpt-live-transcribe", session_id: sid });
  ws.event({ type: "delta", item_id: "B", order: 1, text: "second interim" });
  ws.event({ type: "completed", item_id: "A", order: 0, text: "first" });
  client.stop();
  expect(callbacks.state).toHaveBeenLastCalledWith("finalizing");
  rejectEncoder(
    new DOMException("Context closed during Stop", "InvalidStateError"),
  );
  await vi.advanceTimersByTimeAsync(0);
  expect(callbacks.error).not.toHaveBeenCalled();
  expect(ws.close).not.toHaveBeenCalled();
  expect(callbacks.state).toHaveBeenLastCalledWith("finalizing");
  ws.event({ type: "completed", item_id: "B", order: 1, text: "second" });
  ws.event({ type: "stopped", confirmed: true });
  expect(callbacks.text.mock.lastCall![0].final).toBe("first second");
  expect(callbacks.state).toHaveBeenLastCalledWith("review");
  expect(ws.close).toHaveBeenCalledOnce();
});
it("real PCM startup failure while capture is requested still fails visibly and closes the relay", async () => {
  f.start.mockRejectedValueOnce(
    new DOMException("Cannot start worklet", "NotSupportedError"),
  );
  const { client, callbacks } = fixture();
  await client.start("thread", voicePreferences);
  const ws = Socket.current;
  ws.event({ type: "ready", model: "gpt-live-transcribe", session_id: sid });
  await vi.advanceTimersByTimeAsync(0);
  expect(callbacks.error).toHaveBeenCalledWith(
    "Audio capture is unavailable. Use text or try another browser.",
  );
  expect(callbacks.state).toHaveBeenLastCalledWith("review");
  expect(ws.close).toHaveBeenCalledOnce();
});
