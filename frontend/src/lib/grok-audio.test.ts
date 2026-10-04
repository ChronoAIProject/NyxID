import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { GrokAudio } from "./grok-audio";

const microphone = { getTracks: () => [] } as unknown as MediaStream;
let audio: HTMLAudioElement;
let stop: ReturnType<typeof vi.fn>;
class Peer {
  static all: Peer[] = [];
  localDescription: RTCSessionDescriptionInit = {
    type: "offer",
    sdp: "fixture",
  };
  remoteDescription: RTCSessionDescriptionInit | null = null;
  ontrack?: (e: unknown) => void;
  close = vi.fn();
  addTrack = vi.fn();
  getSenders = () => [{ track: { stop } }];
  createOffer = async () => this.localDescription;
  createAnswer = async () => ({ type: "answer", sdp: "fixture" });
  setLocalDescription = vi.fn(async (value: RTCSessionDescriptionInit) => {
    this.localDescription = value;
  });
  setRemoteDescription = vi.fn(async (value: RTCSessionDescriptionInit) => {
    this.remoteDescription = value;
  });
  config: RTCConfiguration;
  constructor(config: RTCConfiguration) {
    this.config = config;
    Peer.all.push(this);
  }
}
class Context {
  static current: Context;
  state = "running";
  currentTime = 0;
  destination = { speakers: true };
  stream = { getTracks: () => [{ stop }] };
  resume = vi.fn(async () => {});
  close = vi.fn(async () => {});
  source = { connect: vi.fn(), disconnect: vi.fn() };
  destinationNode = { stream: this.stream };
  audioWorklet = { addModule: vi.fn(async () => {}) };
  createMediaStreamSource = () => this.source;
  createMediaStreamDestination = () => this.destinationNode;
  constructor() {
    Context.current = this;
  }
}
class Worklet {
  static current: Worklet;
  port = {
    onmessage: undefined as undefined | ((e: { data: unknown }) => void),
    postMessage: vi.fn(),
    close: vi.fn(),
  };
  connect = vi.fn();
  disconnect = vi.fn();
  constructor() {
    Worklet.current = this;
  }
}
beforeEach(() => {
  Peer.all = [];
  stop = vi.fn();
  vi.stubGlobal("AudioContext", Context);
  vi.stubGlobal("AudioWorkletNode", Worklet);
  vi.stubGlobal("RTCPeerConnection", Peer);
  audio = document.createElement("audio");
  audio.play = vi.fn().mockResolvedValue(undefined);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
it("routes playback through a receive track with no direct speaker path and frees every resource", async () => {
  const pcm = vi.fn();
  const failed = vi.fn();
  const client = new GrokAudio(audio, pcm, failed);
  await client.start(microphone);
  expect(Peer.all).toHaveLength(2);
  expect(Peer.all.every((peer) => peer.config.iceServers?.length === 0)).toBe(
    true,
  );
  expect(Worklet.current.connect).toHaveBeenCalledExactlyOnceWith(
    Context.current.destinationNode,
  );
  const incoming = new MediaStream();
  Peer.all[1]!.ontrack?.({ streams: [incoming], track: {} });
  expect(audio.srcObject).toBe(incoming);
  expect(audio.play).toHaveBeenCalled();
  Worklet.current.port.onmessage?.({
    data: { type: "pcm", generation: 1, bytes: new ArrayBuffer(960) },
  });
  expect(pcm).not.toHaveBeenCalled();
  client.capture(true);
  Worklet.current.port.onmessage?.({
    data: { type: "pcm", generation: 1, bytes: new ArrayBuffer(960) },
  });
  expect(pcm).toHaveBeenCalledOnce();
  client.capture(false);
  client.capture(true);
  Worklet.current.port.onmessage?.({
    data: { type: "pcm", generation: 1, bytes: new ArrayBuffer(960) },
  });
  expect(pcm).toHaveBeenCalledOnce();
  Worklet.current.port.onmessage?.({
    data: { type: "pcm", generation: 3, bytes: new ArrayBuffer(960) },
  });
  expect(pcm).toHaveBeenCalledTimes(2);
  client.close();
  client.close();
  expect(Context.current.close).toHaveBeenCalledOnce();
  expect(Peer.all.every((peer) => peer.close.mock.calls.length === 1)).toBe(
    true,
  );
  expect(Worklet.current.port.close).toHaveBeenCalledOnce();
  expect(Context.current.source.disconnect).toHaveBeenCalledOnce();
  expect(stop).toHaveBeenCalled();
  expect(failed).not.toHaveBeenCalled();
});
it("does not create peers when a closed call's worklet finishes loading", async () => {
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  const client = new GrokAudio(audio, vi.fn(), vi.fn());
  const starting = client.start(microphone);
  Context.current.audioWorklet.addModule.mockReturnValue(pending);
  await Promise.resolve();
  client.close();
  release();
  await starting;
  expect(Peer.all).toHaveLength(0);
  expect(Context.current.close).toHaveBeenCalledOnce();
});

it("advances the playback watermark only when received audio has played and ignores flushed marks", async () => {
  const client = new GrokAudio(audio, vi.fn(), vi.fn());
  await client.start(microphone);
  Peer.all[1]!.ontrack?.({ streams: [new MediaStream()], track: {} });
  Worklet.current.port.onmessage?.({
    data: { type: "played", generation: 0, end_ms: 500, at: 0.1 },
  });
  audio.currentTime = 1;
  expect(client.watermark()).toBe(0); // Paused playback is not heard.
  Object.defineProperty(audio, "paused", { configurable: true, value: false });
  audio.currentTime = 0.1;
  expect(client.watermark()).toBe(0); // Includes the conservative loopback lag.
  audio.currentTime = 1;
  expect(client.watermark()).toBe(500);
  Worklet.current.port.onmessage?.({
    data: { type: "played", generation: 0, end_ms: 700, at: 1 },
  });
  client.flush();
  Worklet.current.port.onmessage?.({
    data: { type: "played", generation: 0, end_ms: 900, at: 1 },
  });
  audio.currentTime = 2;
  expect(client.watermark()).toBe(500);
  client.close();
});
