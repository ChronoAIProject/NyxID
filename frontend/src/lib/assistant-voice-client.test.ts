import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantVoiceClient } from "./assistant-voice-client";
import { ApiError } from "./api-client";
import type { VoicePreferences } from "@/schemas/assistant-voice";
const mocks = vi.hoisted(() => ({
  post: vi.fn(),
  create: vi.fn(),
  capture: vi.fn(),
  enqueue: vi.fn(),
  flush: vi.fn(),
  close: vi.fn(),
  pcm: undefined as undefined | ((bytes: ArrayBuffer) => void),
}));
vi.mock("./grok-audio", () => ({
  GrokAudio: class {
    capture = mocks.capture;
    enqueue = mocks.enqueue;
    flush = mocks.flush;
    close = mocks.close;
    start = vi.fn(async () => {});
    resume = vi.fn(async () => {});
    watermark = () => 120;
    constructor(_audio: HTMLAudioElement, pcm: (bytes: ArrayBuffer) => void) {
      mocks.pcm = pcm;
    }
  },
}));
vi.mock("./api-client", async (original) => ({
  ...(await original<typeof import("./api-client")>()),
  api: { post: mocks.post },
  apiClient: mocks.create,
}));
const session = {
  id: "f985604b-1991-4aa2-969d-2325055ea813",
  generation: 1,
  state: "active",
  muted: false,
  input_muted: false,
  end_requested: false,
  control_revision: 0,
  observed_seconds: 0,
  reserved_until: 30,
  final_usage_confirmed: false,
  end_reason: null,
  idle_warning: false,
  created_at: "2026-10-03T00:00:00Z",
  closed_at: null,
};
const preferences: VoicePreferences = {
  service_id: crypto.randomUUID(),
  connection_id: crypto.randomUUID(),
  key_source: "own",
  model: "gpt-live-1",
  voice: "marin",
  input_mode: "automatic",
  language: null,
  notify_on_completion: false,
};
class Peer {
  static current: Peer;
  connectionState = "new";
  ontrack?: (e: unknown) => void;
  onconnectionstatechange?: () => void;
  localDescription = { sdp: "v=0\r\nm=audio" };
  channel = {
    onmessage: undefined as undefined | ((e: { data: string }) => void),
    send: vi.fn(),
  };
  close = vi.fn();
  addTrack = vi.fn();
  createOffer = vi.fn(async () => this.localDescription);
  setLocalDescription = vi.fn(async () => {});
  setRemoteDescription = vi.fn(async () => {});
  createDataChannel = vi.fn(() => this.channel);
  constructor() {
    Peer.current = this;
  }
}
class Socket {
  static OPEN = 1;
  static current: Socket;
  readyState = 1;
  onmessage?: (e: { data: string | ArrayBuffer }) => void;
  onclose?: () => void;
  send = vi.fn();
  close = vi.fn();
  constructor() {
    Socket.current = this;
  }
  snapshot() {
    this.onmessage?.({
      data: JSON.stringify({
        type: "snapshot",
        session,
        captions: [],
        tasks: [],
      }),
    });
  }
}
let track: { enabled: boolean; stop: ReturnType<typeof vi.fn> };
let microphone: ReturnType<typeof vi.fn>;
let audio: HTMLAudioElement;
beforeEach(() => {
  vi.useFakeTimers();
  mocks.capture.mockReset();
  mocks.enqueue.mockReset();
  mocks.flush.mockReset();
  mocks.close.mockReset();
  mocks.create.mockReset();
  mocks.post.mockReset();
  track = { enabled: true, stop: vi.fn() };
  microphone = vi.fn().mockResolvedValue({
    getAudioTracks: () => [track],
    getTracks: () => [track],
  });
  vi.stubGlobal("RTCPeerConnection", Peer);
  vi.stubGlobal("WebSocket", Socket);
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: { getUserMedia: microphone },
  });
  const create = document.createElement.bind(document);
  vi.spyOn(document, "createElement").mockImplementation((name, options) => {
    const element = create(name, options);
    if (name === "audio") {
      audio = element as HTMLAudioElement;
      audio.play = vi.fn().mockResolvedValue(undefined);
      audio.pause = vi.fn();
    }
    return element;
  });
  mocks.create.mockResolvedValue({ session, sdp_answer: "v=0\r\nm=audio" });
  mocks.post.mockResolvedValue({ ...session, muted: true });
});
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
describe("voice media boundaries", () => {
  it("waits for both authorities, keeps speaker mute local, and never sends provider commands", async () => {
    const closed = vi.fn();
    const client = new AssistantVoiceClient("thread", vi.fn(), vi.fn(), closed);
    await client.start(preferences);
    expect(track.enabled).toBe(false);
    Peer.current.channel.onmessage?.({
      data: JSON.stringify({ type: "session.started" }),
    });
    Socket.current.snapshot();
    expect(track.enabled).toBe(true);
    client.muteSpeaker(true);
    expect(audio.muted).toBe(true);
    expect(track.enabled).toBe(true);
    expect(mocks.post).not.toHaveBeenCalled();
    await client.mute(true);
    expect(track.enabled).toBe(false);
    expect(audio.muted).toBe(true);
    client.muteSpeaker(false);
    expect(audio.muted).toBe(false);
    expect(track.enabled).toBe(false);
    Peer.current.channel.onmessage?.({
      data: JSON.stringify({
        type: "response.function_call_arguments.done",
        name: "ask_compute",
        arguments: "the user approved",
      }),
    });
    expect(Peer.current.channel.send).not.toHaveBeenCalled();
    await client.end();
    expect(track.stop).toHaveBeenCalled();
    expect(closed).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(5000);
    expect(Peer.current.close).toHaveBeenCalled();
    expect(Socket.current.close).toHaveBeenCalled();
    expect(audio.srcObject).toBeNull();
  });
  it("cleans up microphone permission resolving after end without creating a call", async () => {
    let resolve!: (value: unknown) => void;
    microphone.mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }),
    );
    const client = new AssistantVoiceClient("thread", vi.fn(), vi.fn());
    const starting = client.start(preferences);
    await client.end();
    resolve({ getTracks: () => [track], getAudioTracks: () => [track] });
    await starting;
    expect(track.stop).toHaveBeenCalled();
    expect(mocks.create).not.toHaveBeenCalled();
  });
  it("shows the server's single-call conflict and releases local capture", async () => {
    mocks.create.mockRejectedValue(
      new ApiError(409, {
        error: "conflict",
        error_code: 1009,
        message: "End the current call first",
      }),
    );
    const error = vi.fn();
    const client = new AssistantVoiceClient("thread", vi.fn(), error);
    await client.start(preferences);
    expect(error).toHaveBeenCalledWith("End the current call first");
    expect(client.isClosed).toBe(true);
    expect(track.stop).toHaveBeenCalled();
  });
});

it("Grok relays only held PCM, pairs bounded output, and commits before mic mute", async () => {
  const client = new AssistantVoiceClient("thread", vi.fn(), vi.fn());
  await client.start(
    { ...preferences, input_mode: "push_to_talk" },
    "xai_realtime",
  );
  expect(mocks.create.mock.calls[0]![1].body.sdp_offer).toBe("");
  expect(track.enabled).toBe(false);
  Socket.current.snapshot();
  client.hold(true);
  expect(track.enabled).toBe(true);
  expect(Socket.current.send).toHaveBeenCalledWith(
    JSON.stringify({ type: "ptt_begin" }),
  );
  const pcm = new ArrayBuffer(960);
  mocks.pcm?.(pcm);
  expect(Socket.current.send).toHaveBeenCalledWith(pcm);
  Socket.current.onmessage?.({
    data: JSON.stringify({ type: "audio", generation: 1, end_ms: 100 }),
  });
  Socket.current.onmessage?.({ data: pcm });
  expect(mocks.enqueue).toHaveBeenCalledWith(pcm, 100);
  client.muteSpeaker(true);
  expect(track.enabled).toBe(true);
  await client.mute(true);
  expect(Socket.current.send).toHaveBeenCalledWith(
    JSON.stringify({ type: "ptt_commit" }),
  );
  expect(track.enabled).toBe(false);
  const sent = Socket.current.send.mock.calls.length;
  mocks.pcm?.(pcm);
  expect(Socket.current.send.mock.calls).toHaveLength(sent);
  await client.end();
  Socket.current.onmessage?.({ data: pcm });
  expect(mocks.enqueue).toHaveBeenCalledTimes(1);
  expect(mocks.close).toHaveBeenCalled();
});
it("Grok refuses automatic mode before creating a provider session", async () => {
  const client = new AssistantVoiceClient("thread", vi.fn(), vi.fn());
  await client.start(preferences, "xai_realtime");
  expect(mocks.create).not.toHaveBeenCalled();
  expect(track.stop).toHaveBeenCalled();
});
