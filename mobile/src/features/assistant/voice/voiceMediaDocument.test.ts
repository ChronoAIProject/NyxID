import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import { deferred, hookHarness } from "../assistantHookTestHarness";
import type {
  VoiceMediaAdapter,
  VoiceMediaEvent,
} from "./AssistantVoiceClient";
import * as documentModule from "./voiceMediaDocument";
import {
  createVoiceMediaDocument,
  VOICE_MEDIA_ENGINE,
  VOICE_WORKLET_SOURCE,
  voicePcmFromBase64,
  voicePcmToBase64,
} from "./voiceMediaDocument";

async function settle() {
  for (let i = 0; i < 8; i++)
    await new Promise<void>((yes) => setImmediate(yes));
}

type Media = VoiceMediaAdapter & {
  receive(raw: string): void;
  unavailable(message?: string): void;
  connect(send: (message: Record<string, unknown>) => void): () => void;
  getDiagnostics(): readonly {
    stage: string;
    name: string;
    counts?: Record<string, number>;
  }[];
};

function nativeMedia(): Media {
  const harness = hookHarness<unknown>({
    "react-native": { StyleSheet: { create: (value: unknown) => value } },
    "react-native-webview": { WebView: "WebView" },
    "./voiceMediaDocument": documentModule,
  });
  const module = harness.load(
    fileURLToPath(new URL("./VoiceMediaBridge.tsx", import.meta.url)),
  ) as { NativeVoiceMedia: new () => Media };
  return new module.NativeVoiceMedia();
}

function engine(
  options: {
    secure?: boolean;
    label?: string;
    media?: Promise<unknown> | (() => Promise<unknown>);
    play?: () => Promise<void>;
    stats?: () => Promise<Map<string, Record<string, unknown>>>;
    receive?: (raw: string) => void;
    iceComplete?: boolean;
    loopbackConnected?: boolean;
    candidateSDP?: string;
    /** Give AudioContext analysers whose waveform amplitude tests can set. */
    analysers?: boolean;
  } = {},
) {
  const messages: Record<string, any>[] = [];
  const peers: Peer[] = [];
  const tracks = [
    {
      ...(options.label === undefined ? {} : { label: options.label }),
      enabled: true,
      readyState: "live",
      muted: false,
      stopped: false,
      listeners: new Map<string, () => void>(),
      addEventListener(type: string, callback: () => void) {
        this.listeners.set(type, callback);
      },
      removeEventListener(type: string) {
        this.listeners.delete(type);
      },
      stop() {
        this.stopped = true;
        this.readyState = "ended";
      },
    },
  ];
  const stream = { getTracks: () => tracks, getAudioTracks: () => tracks };
  const timers = new Map<number, () => void>();
  const intervals = new Map<number, { callback: () => void; delay: number }>();
  const documentListeners = new Map<string, () => void>();
  let timerId = 0;
  let micRequests = 0;
  let statsRequests = 0;
  const audio = {
    autoplay: false,
    currentTime: 0,
    paused: true,
    muted: false,
    volume: 1,
    readyState: 4,
    srcObject: null as unknown,
    removed: false,
    setAttribute() {},
    async play() {
      await options.play?.();
      this.paused = false;
    },
    pause() {
      this.paused = true;
    },
    remove() {
      this.removed = true;
    },
  };
  class Peer {
    localDescription?: { type: string; sdp: string };
    remoteDescription?: unknown;
    iceGatheringState =
      options.iceComplete === false ? "gathering" : "complete";
    connectionState = options.loopbackConnected === false ? "new" : "connected";
    iceConnectionState = "connected";
    closed = false;
    addedCandidates = 0;
    channel = {
      label: "",
      readyState: "open",
      onmessage: (_: { data: string }) => {},
      onopen: () => {},
      onclose: () => {},
      onerror: () => {},
    };
    ontrack?: (event: unknown) => void;
    oniceconnectionstatechange?: () => void;
    onconnectionstatechange?: () => void;
    listeners = new Map<string, () => void>();
    constructor() {
      peers.push(this);
    }
    addTrack() {}
    async getStats() {
      statsRequests++;
      return options.stats ? await options.stats() : new Map();
    }
    createDataChannel(label: string) {
      this.channel.label = label;
      return this.channel;
    }
    async createOffer() {
      return {
        type: "offer",
        sdp: "v=0\r\nmock-offer\r\n" + (options.candidateSDP ?? ""),
      };
    }
    async createAnswer() {
      return {
        type: "answer",
        sdp: "v=0\r\nmock-answer\r\n" + (options.candidateSDP ?? ""),
      };
    }
    async setLocalDescription(value: { type: string; sdp: string }) {
      this.localDescription = value;
    }
    async setRemoteDescription(value: unknown) {
      this.remoteDescription = value;
    }
    async addIceCandidate() {
      this.addedCandidates++;
    }
    addEventListener(type: string, handler: () => void) {
      this.listeners.set(type, handler);
    }
    removeEventListener(type: string) {
      this.listeners.delete(type);
    }
    close() {
      this.closed = true;
    }
  }
  const worklets: any[] = [];
  const contexts: any[] = [];
  const analysers: any[] = [];
  class Analyser {
    fftSize = 2048;
    smoothingTimeConstant = 0.8;
    amplitude = 0;
    inputs: unknown[] = [];
    connections: unknown[] = [];
    disconnected = false;
    constructor(public context: unknown) {
      analysers.push(this);
    }
    getFloatTimeDomainData(samples: Float32Array) {
      // A square wave: its RMS is exactly the amplitude.
      for (let i = 0; i < samples.length; i++)
        samples[i] = i % 2 ? this.amplitude : -this.amplitude;
    }
    connect(value: unknown) {
      this.connections.push(value);
    }
    disconnect() {
      this.disconnected = true;
    }
  }
  class Context {
    state = "suspended";
    currentTime = 0;
    closed = false;
    destination = {};
    audioWorklet = { addModule: async (_: string) => {} };
    constructor() {
      contexts.push(this);
    }
    async resume() {
      this.state = "running";
    }
    async close() {
      this.closed = true;
      this.state = "closed";
    }
    createMediaStreamSource(stream: unknown) {
      return {
        stream,
        connect(node: any) {
          node?.inputs?.push(this);
        },
        disconnect() {},
      };
    }
    createMediaStreamDestination() {
      return { stream, connect() {}, disconnect() {} };
    }
  }
  class Worklet {
    commands: any[] = [];
    connections: unknown[] = [];
    port = {
      onmessage: (_: { data: unknown }) => {},
      postMessage: (message: unknown) => this.commands.push(message),
      close: () => {},
    };
    constructor() {
      worklets.push(this);
    }
    connect(value: any) {
      this.connections.push(value);
      value?.inputs?.push(this);
    }
    disconnect() {}
  }
  if (options.analysers)
    (Context.prototype as any).createAnalyser = function () {
      return new Analyser(this);
    };
  const sandbox: any = {
    isSecureContext: options.secure !== false,
    navigator: {
      mediaDevices: {
        getUserMedia: async () => {
          micRequests++;
          return typeof options.media === "function"
            ? await options.media()
            : options.media
              ? await options.media
              : stream;
        },
      },
    },
    document: {
      hidden: false,
      body: { appendChild() {} },
      createElement: () => audio,
      addEventListener: (type: string, listener: () => void) =>
        documentListeners.set(type, listener),
    },
    ReactNativeWebView: {
      postMessage: (raw: string) => {
        messages.push(JSON.parse(raw));
        options.receive?.(raw);
      },
    },
    RTCPeerConnection: Peer,
    AudioContext: Context,
    AudioWorkletNode: Worklet,
    MediaStream: class {
      constructor(_: unknown) {}
    },
    ArrayBuffer,
    Uint8Array,
    DataView,
    Float32Array,
    Set,
    Blob,
    URL: {
      createObjectURL: () => "blob:local-processor",
      revokeObjectURL() {},
    },
    btoa: (value: string) => Buffer.from(value, "binary").toString("base64"),
    atob: (value: string) => Buffer.from(value, "base64").toString("binary"),
    setTimeout: (callback: () => void) => {
      const id = ++timerId;
      timers.set(id, callback);
      return id;
    },
    clearTimeout: (id: number) => timers.delete(id),
    setInterval: (callback: () => void, delay: number) => {
      const id = ++timerId;
      intervals.set(id, { callback, delay });
      return id;
    },
    clearInterval: (id: number) => intervals.delete(id),
    addEventListener() {},
    __nyxVoiceWorklet: VOICE_WORKLET_SOURCE,
    Math,
  };
  sandbox.window = sandbox;
  runInNewContext(VOICE_MEDIA_ENGINE, sandbox);
  return {
    command: (message: Record<string, unknown>) =>
      sandbox.__nyxVoiceMessage(message),
    messages,
    peers,
    tracks,
    stream,
    audio,
    worklets,
    contexts,
    analysers,
    timers,
    intervals,
    tick: () => [...intervals.values()].forEach(({ callback }) => callback()),
    tickInterval: (delay: number) =>
      [...intervals.values()]
        .filter((interval) => interval.delay === delay)
        .forEach(({ callback }) => callback()),
    micRequests: () => micRequests,
    statsRequests: () => statsRequests,
    hide: () => {
      sandbox.document.hidden = true;
      documentListeners.get("visibilitychange")?.();
    },
  };
}

test("the media document contains only bundled code and no credentials or network client", () => {
  const html = createVoiceMediaDocument();
  assert.match(html, /connect-src 'none'/);
  assert.doesNotMatch(
    html,
    /<script[^>]+src=|https?:\/\/|fetch\(|WebSocket\(|localStorage|Authorization/,
  );
  assert.equal((html.match(/<\/script>/g) ?? []).length, 1);
  const actual = readFileSync(
    new URL(
      "../../../../../frontend/src/lib/grok-audio-worklet.js",
      import.meta.url,
    ),
    "utf8",
  );
  assert.equal(VOICE_WORKLET_SOURCE, actual);
});

test("bounded PCM base64 roundtrips and rejects malformed/oversized frames", () => {
  for (const size of [2, 4, 960, 1920]) {
    const bytes = Uint8Array.from({ length: size }, (_, i) => i % 256);
    const encoded = voicePcmToBase64(bytes.buffer);
    assert.equal(encoded, Buffer.from(bytes).toString("base64"));
    assert.deepEqual(new Uint8Array(voicePcmFromBase64(encoded)), bytes);
  }
  for (const size of [0, 1, 1922])
    assert.throws(() => voicePcmToBase64(new ArrayBuffer(size)));
  for (const value of ["", "AA==", "!!!!", "A==A", "AA", "A".repeat(2564)])
    assert.throws(() => voicePcmFromBase64(value));
});

test("insecure contexts fail before acquiring the microphone", async () => {
  const e = engine({ secure: false });
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.micRequests(), 0);
  assert.match(
    e.messages.find((m) => m.type === "error")?.message,
    /not supported/,
  );
});

test("known WebKit mock microphones reject both voice transports before any peer or offer", async () => {
  const reason =
    "A test microphone is active. Select a real microphone and try again.";
  for (const label of [
    "Mock audio device 1",
    "Mock audio device 2",
    "Mock audio device 3",
    "Mock audio device 4",
  ]) {
    for (const protocol of ["openai_live", "xai_realtime"] as const) {
      const media = nativeMedia();
      const events: VoiceMediaEvent[] = [];
      media.subscribe((event) => events.push(event));
      const e = engine({ label, receive: (raw) => media.receive(raw) });
      media.connect(e.command);
      await assert.rejects(
        media.prepare(protocol, new AbortController().signal),
        { message: reason },
      );
      assert.equal(e.tracks[0]!.stopped, true);
      assert.equal(e.tracks[0]!.enabled, false);
      assert.equal(e.peers.length, 0);
      assert.equal(e.contexts.length, 0);
      assert.equal(e.worklets.length, 0);
      assert.equal(
        e.messages.some(
          (message) =>
            message.type === "response" && typeof message.result === "string",
        ),
        false,
      );
      assert.equal(
        e.messages.find((message) => message.type === "error")?.message,
        reason,
      );
      assert.equal(
        events.find((event) => event.type === "error")?.message,
        reason,
      );
      assert.equal(
        media.getDiagnostics().find((row) => row.stage === "input_source")
          ?.counts?.mock_capture,
        1,
      );
      assert.equal(e.audio.removed, true);
      assert.equal(
        JSON.stringify([
          ...e.messages,
          ...events,
          ...media.getDiagnostics(),
        ]).includes(label),
        false,
      );
      media.close();
    }
  }
});

test("missing, physical and near-match microphone labels remain eligible without exposing labels", async () => {
  for (const label of [
    undefined,
    "",
    "Built-in Microphone",
    "Mock audio device 0",
    "Mock audio device 5",
    "My Mock audio device 1",
    "Mock audio device 1 ",
    "Mock audio device 1\n",
    "mock audio device 1",
  ]) {
    const e = engine({ label });
    e.command({
      type: "prepare",
      protocol: "openai_live",
      generation: 1,
      requestId: 1,
    });
    await settle();
    assert.equal(e.peers.length, 1);
    assert.equal(e.tracks[0]!.stopped, false);
    assert.match(
      e.messages.find((message) => message.type === "response")?.result,
      /^v=0/,
    );
    assert.equal(
      e.messages.some((message) => message.type === "error"),
      false,
    );
    assert.equal(
      e.messages.find((message) => message.stage === "input_source")?.counts
        ?.mock_capture,
      0,
    );
    if (label) assert.equal(JSON.stringify(e.messages).includes(label), false);
    assert.doesNotMatch(JSON.stringify(e.messages), /"label"|"deviceId"/);
    e.command({ type: "close", generation: 1 });
  }
});

test("a mock second audio track rejects capture and stops the entire stream", async () => {
  const mockTrack = {
    label: "Mock audio device 4",
    enabled: true,
    stopped: false,
    stop() {
      this.stopped = true;
    },
  };
  const e = engine({
    media: async () => ({
      getAudioTracks: () => [...e.tracks, mockTrack],
      getTracks: () => [...e.tracks, mockTrack],
    }),
  });
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(mockTrack.stopped, true);
  assert.equal(e.peers.length, 0);
  assert.equal(
    e.messages.find((message) => message.type === "response")?.error,
    "A test microphone is active. Select a real microphone and try again.",
  );
  assert.equal(JSON.stringify(e.messages).includes(mockTrack.label), false);
});

test("input source diagnostics keep only the bounded mock-capture flag", async () => {
  const media = nativeMedia();
  const e = engine({ receive: (raw) => media.receive(raw) });
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  for (const mock_capture of [-1, 0.5, 2, "1"]) {
    media.receive(
      JSON.stringify({
        type: "diagnostic",
        generation: 1,
        stage: "input_source",
        name: "ready",
        counts: {
          mock_capture,
          label: "private device label",
          deviceId: "private id",
        },
      }),
    );
    assert.equal(
      media.getDiagnostics().at(-1)?.counts?.mock_capture,
      undefined,
    );
  }
  media.receive(
    JSON.stringify({
      type: "diagnostic",
      generation: 1,
      stage: "input_source",
      name: "ready",
      counts: {
        mock_capture: 1,
        label: "private device label",
        deviceId: "private id",
      },
    }),
  );
  assert.deepEqual(
    { ...media.getDiagnostics().at(-1)?.counts },
    { mock_capture: 1 },
  );
  assert.doesNotMatch(
    JSON.stringify(media.getDiagnostics()),
    /private|deviceId|label/,
  );
  media.close();
});

test("OpenAI starts muted, negotiates SDP, and exposes only provider readiness", async () => {
  const e = engine();
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.tracks[0]!.enabled, false);
  assert.equal(e.peers[0]!.channel.label, "oai-events");
  assert.match(e.messages.find((m) => m.type === "response")?.result, /^v=0/);
  e.peers[0]!.channel.onmessage({
    data: JSON.stringify({ type: "secret-provider-event", message: "private" }),
  });
  e.peers[0]!.channel.onmessage({
    data: JSON.stringify({ type: "session.started" }),
  });
  assert.equal(e.messages.filter((m) => m.type === "provider-ready").length, 1);
  assert.doesNotMatch(JSON.stringify(e.messages), /private/);
  e.command({ type: "input", enabled: true, generation: 1 });
  assert.equal(e.tracks[0]!.enabled, true);
  e.command({ type: "close", generation: 1 });
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(e.peers[0]!.closed, true);
  assert.equal(e.audio.removed, true);
});

test("closing while mic permission is pending stops the late stream without negotiating", async () => {
  const permission = deferred<unknown>();
  const e = engine({ media: permission.promise });
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  e.command({ type: "close", generation: 1 });
  permission.resolve(e.stream);
  await settle();
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(e.peers.length, 0);
  assert.equal(e.messages.filter((m) => m.type === "response").length, 0);
});

test("ICE gathering has a five-second deadline and releases tracks on failure", async () => {
  const e = engine({ iceComplete: false });
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.timers.size, 1);
  [...e.timers.values()][0]!();
  await settle();
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(e.peers[0]!.closed, true);
  assert.equal(e.messages.filter((m) => m.type === "error").length, 1);
});

test("Grok preserves web loopback playback and fences capture and output generations", async () => {
  const e = engine();
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 3,
    requestId: 1,
  });
  await settle();
  assert.equal(e.peers.length, 2);
  const worklet = e.worklets[0]!;
  assert.equal(worklet.connections.length, 1);
  assert.notEqual(worklet.connections[0], e.contexts[0]!.destination);
  assert.equal(e.tracks[0]!.enabled, false);
  e.command({ type: "input", enabled: true, generation: 3 });
  const capture = worklet.commands.find((m: any) => m.type === "capture");
  worklet.port.onmessage({
    data: {
      type: "pcm",
      generation: capture.generation - 1,
      bytes: new ArrayBuffer(960),
    },
  });
  assert.equal(e.messages.filter((m) => m.type === "pcm").length, 0);
  worklet.port.onmessage({
    data: {
      type: "pcm",
      generation: capture.generation,
      bytes: new ArrayBuffer(960),
    },
  });
  assert.equal(e.messages.filter((m) => m.type === "pcm").length, 1);
  e.command({
    type: "enqueue",
    generation: 3,
    bytes: voicePcmToBase64(new ArrayBuffer(960)),
    end_ms: 20,
  });
  assert.equal(
    worklet.commands.find((m: any) => m.type === "audio").generation,
    0,
  );
  e.command({ type: "flush", generation: 3 });
  e.command({
    type: "enqueue",
    generation: 3,
    bytes: voicePcmToBase64(new ArrayBuffer(960)),
    end_ms: 40,
  });
  assert.equal(
    worklet.commands.filter((m: any) => m.type === "audio")[1].generation,
    1,
  );
  e.command({ type: "close", generation: 3 });
  assert.equal(e.contexts[0]!.closed, true);
  assert.equal(
    e.peers.every((peer) => peer.closed),
    true,
  );
});

test("Grok exchanges complete SDP once and never trickles duplicate candidates", async () => {
  const e = engine({ iceComplete: false });
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 1,
    requestId: 1,
  });
  await settle();
  const sender = e.peers[0]!;
  const receiver = e.peers[1]!;
  assert.equal(receiver.remoteDescription, undefined);
  sender.iceGatheringState = "complete";
  sender.listeners.get("icegatheringstatechange")!();
  await settle();
  assert.ok(receiver.remoteDescription);
  assert.equal(sender.remoteDescription, undefined);
  receiver.iceGatheringState = "complete";
  receiver.listeners.get("icegatheringstatechange")!();
  await settle();
  assert.ok(sender.remoteDescription);
  assert.equal(sender.addedCandidates + receiver.addedCandidates, 0);
  assert.equal(
    e.messages.filter((message) => message.type === "response").length,
    1,
  );
  e.command({ type: "close", generation: 1 });
});

test("loopback diagnostics report only candidate counts and types, without addresses or SDP", async () => {
  const media = nativeMedia();
  const e = engine({
    receive: (raw) => media.receive(raw),
    candidateSDP:
      "a=candidate:abc 1 udp 123 synthetic-host.local 12345 typ host\r\na=candidate:def 1 tcp 123 192.0.2.55 23456 typ host\r\n",
  });
  media.connect(e.command);
  await media.prepare("xai_realtime", new AbortController().signal);
  const candidates = media
    .getDiagnostics()
    .filter((row) => row.counts?.candidates !== undefined);
  assert.equal(candidates.length, 2);
  for (const row of candidates) {
    assert.equal(row.counts?.candidates, 2);
    assert.equal(row.counts?.host, 2);
    assert.equal(row.counts?.mdns, 1);
    assert.equal(row.counts?.udp, 1);
    assert.equal(row.counts?.tcp, 1);
  }
  assert.doesNotMatch(
    JSON.stringify(media.getDiagnostics()),
    /synthetic-host|192\.0\.2|12345|23456|candidate:/,
  );
  media.close();
});

test("Grok preparation waits for the receive connection before reporting media ready", async () => {
  const e = engine({ loopbackConnected: false });
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(
    e.messages.filter((message) => message.type === "response").length,
    0,
  );
  assert.equal(e.timers.size, 1);
  const receiver = e.peers[1]!;
  receiver.connectionState = "connected";
  receiver.listeners.get("connectionstatechange")!();
  await settle();
  assert.equal(
    e.messages.filter((message) => message.type === "response").length,
    1,
  );
  assert.equal(
    e.messages.find((message) => message.type === "response")?.result,
    "",
  );
  assert.equal(e.timers.size, 0);
  e.command({ type: "close", generation: 1 });
});

test("Grok's five-second connection deadline closes all audio and peer resources", async () => {
  const e = engine({ loopbackConnected: false });
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.timers.size, 1);
  [...e.timers.values()][0]!();
  await settle();
  assert.match(
    e.messages.find((message) => message.type === "error")?.message,
    /playback could not be started/,
  );
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(
    e.peers.every((peer) => peer.closed),
    true,
  );
  assert.equal(e.contexts[0]!.closed, true);
  assert.equal(e.timers.size, 0);
});

test("closing during Grok connection startup removes the waiter and ignores late connection events", async () => {
  const e = engine({ loopbackConnected: false });
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 1,
    requestId: 1,
  });
  await settle();
  const receiver = e.peers[1]!;
  const lateConnected = receiver.listeners.get("connectionstatechange")!;
  e.command({ type: "close", generation: 1 });
  await settle();
  assert.equal(e.timers.size, 0);
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(
    e.peers.every((peer) => peer.closed),
    true,
  );
  assert.equal(e.contexts[0]!.closed, true);
  receiver.connectionState = "connected";
  lateConnected();
  await settle();
  assert.equal(
    e.messages.filter(
      (message) => message.type === "response" || message.type === "error",
    ).length,
    0,
  );
});

test("becoming inactive during Grok connection startup tears down its media immediately", async () => {
  const e = engine({ loopbackConnected: false });
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 1,
    requestId: 1,
  });
  await settle();
  e.hide();
  await settle();
  assert.match(
    e.messages.find((message) => message.type === "error")?.message,
    /foreground/,
  );
  assert.equal(e.timers.size, 0);
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(
    e.peers.every((peer) => peer.closed),
    true,
  );
  assert.equal(e.contexts[0]!.closed, true);
});

test("native abort while awaiting Grok's loopback rejects preparation without leaking media", async () => {
  const media = nativeMedia();
  const e = engine({
    loopbackConnected: false,
    receive: (raw) => media.receive(raw),
  });
  media.connect(e.command);
  const abort = new AbortController();
  const prepared = media.prepare("xai_realtime", abort.signal);
  const rejected = assert.rejects(prepared, /closed/);
  await settle();
  abort.abort();
  await rejected;
  await settle();
  assert.equal(e.timers.size, 0);
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(
    e.peers.every((peer) => peer.closed),
    true,
  );
  assert.equal(e.contexts[0]!.closed, true);
});

test("Grok closes when the RN bridge cannot acknowledge PCM within its bounded window", async () => {
  const e = engine();
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 1,
    requestId: 1,
  });
  await settle();
  e.command({ type: "input", enabled: true, generation: 1 });
  const worklet = e.worklets[0]!;
  const capture = worklet.commands.find((m: any) => m.type === "capture");
  for (let i = 0; i < 101; i++)
    worklet.port.onmessage({
      data: {
        type: "pcm",
        generation: capture.generation,
        bytes: new ArrayBuffer(960),
      },
    });
  assert.equal(e.messages.filter((m) => m.type === "pcm").length, 100);
  assert.match(e.messages.find((m) => m.type === "error")?.message, /too slow/);
  assert.equal(e.tracks[0]!.stopped, true);
});

test("native adapter and real engine exchange SDP and ignore old-generation callbacks", async () => {
  const media = nativeMedia();
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  const e = engine({ receive: (raw) => media.receive(raw) });
  const disconnect = media.connect(e.command);
  const offer = await media.prepare(
    "openai_live",
    new AbortController().signal,
  );
  assert.match(offer, /^v=0/);
  await media.answer("v=0\r\nserver-answer");
  assert.equal(
    JSON.stringify(e.peers[0]!.remoteDescription),
    JSON.stringify({ type: "answer", sdp: "v=0\r\nserver-answer" }),
  );
  media.setInput(true);
  assert.equal(e.tracks[0]!.enabled, true);
  e.peers[0]!.channel.onmessage({ data: '{"type":"session.started"}' });
  assert.equal(events[0]?.type, "provider-ready");
  media.receive(
    JSON.stringify({ type: "error", generation: 99, message: "private" }),
  );
  assert.equal(events.length, 1);
  media.close();
  assert.equal(e.tracks[0]!.stopped, true);
  media.setInput(true);
  assert.equal(e.tracks[0]!.stopped, true);
  disconnect();
});

test("abort before document readiness rejects immediately and prevents a late start", async () => {
  const media = nativeMedia();
  const commands: unknown[] = [];
  media.connect((command) => commands.push(command));
  const abort = new AbortController();
  const result = media.prepare("openai_live", abort.signal);
  const rejected = assert.rejects(result, /closed/);
  abort.abort();
  await rejected;
  media.receive('{"type":"ready"}');
  assert.equal(commands.length, 0);
});

test("abort after mic capture starts closes late engine resources and rejects the pending offer", async () => {
  const permission = deferred<unknown>();
  const media = nativeMedia();
  const e = engine({
    media: permission.promise,
    receive: (raw) => media.receive(raw),
  });
  media.connect(e.command);
  const abort = new AbortController();
  const result = media.prepare("openai_live", abort.signal);
  const rejected = assert.rejects(result, /closed/);
  await settle();
  abort.abort();
  await rejected;
  permission.resolve(e.stream);
  await settle();
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(e.peers.length, 0);
});

test("Grok bridge acknowledges PCM and local speaker mute updates playback telemetry", async () => {
  const media = nativeMedia();
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  const e = engine({ receive: (raw) => media.receive(raw) });
  media.connect(e.command);
  assert.equal(
    await media.prepare("xai_realtime", new AbortController().signal),
    "",
  );
  media.setInput(true);
  const worklet = e.worklets[0]!;
  const capture = worklet.commands.find((m: any) => m.type === "capture");
  for (let i = 0; i < 110; i++)
    worklet.port.onmessage({
      data: {
        type: "pcm",
        generation: capture.generation,
        bytes: new ArrayBuffer(960),
      },
    });
  assert.equal(events.filter((event) => event.type === "pcm").length, 110);
  e.audio.srcObject = e.stream;
  await media.resume();
  e.tick();
  assert.equal(media.playback().playback_audible, true);
  media.muteSpeaker(true);
  e.tick();
  assert.equal(media.playback().playback_audible, false);
  media.close();
  assert.equal(media.playback().playback_ms, 0);
});

test("a terminated native transport rejects pending work and cleanup stays idempotent", async () => {
  const media = nativeMedia();
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  media.connect(() => {
    throw new Error("private native error");
  });
  media.receive('{"type":"ready"}');
  await assert.rejects(
    media.prepare("openai_live", new AbortController().signal),
    /closed/,
  );
  assert.equal(events.filter((event) => event.type === "error").length, 1);
  assert.doesNotMatch(JSON.stringify(events), /private/);
  assert.doesNotThrow(() => media.close());
  assert.doesNotThrow(() => media.setInput(true));
});

test("media diagnostics keep only bounded stage and exception-name enums", async () => {
  const media = nativeMedia();
  const e = engine({ receive: (raw) => media.receive(raw) });
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  for (let i = 0; i < 40; i++)
    media.receive(
      JSON.stringify({
        type: "diagnostic",
        generation: 1,
        stage: "loopback_connected",
        name: "OperationError",
        message: "private provider error",
      }),
    );
  media.receive(
    JSON.stringify({
      type: "diagnostic",
      generation: 1,
      stage: "private IP",
      name: "private error",
    }),
  );
  const diagnostics = media.getDiagnostics();
  assert.equal(diagnostics.length, 32);
  assert.equal(
    diagnostics.every(
      (row) =>
        row.stage === "loopback_connected" && row.name === "OperationError",
    ),
    true,
  );
  assert.doesNotMatch(JSON.stringify(diagnostics), /private/);
  media.close();
});

test("OpenAI playback rejection offers recovery without ending capture, including a failed Resume", async () => {
  const media = nativeMedia();
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  let blocked = true;
  const e = engine({
    receive: (raw) => media.receive(raw),
    play: async () => {
      if (blocked)
        throw new DOMException("private playback details", "NotAllowedError");
    },
  });
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  e.peers[0]!.ontrack?.({ track: {}, streams: [e.stream] });
  await settle();
  assert.equal(
    events.filter((event) => event.type === "playback-blocked").length,
    1,
  );
  assert.equal(
    events.some((event) => event.type === "error"),
    false,
  );
  assert.equal(e.tracks[0]!.stopped, false);
  assert.equal(e.peers[0]!.closed, false);
  assert.equal(
    media
      .getDiagnostics()
      .some(
        (row) => row.stage === "openai_play" && row.name === "NotAllowedError",
      ),
    true,
  );
  await assert.rejects(media.resume(), /Tap Resume audio/);
  assert.equal(e.tracks[0]!.stopped, false);
  blocked = false;
  await media.resume();
  assert.equal(e.audio.paused, false);
  assert.equal(
    media
      .getDiagnostics()
      .some((row) => row.stage === "openai_play" && row.name === "ready"),
    true,
  );
  assert.doesNotMatch(
    JSON.stringify([...media.getDiagnostics(), ...events]),
    /private/,
  );
  media.close();
});

test("microphone mute is diagnostic, while ended closes tracks, peers, listeners and samplers", async () => {
  const media = nativeMedia();
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  const e = engine({ receive: (raw) => media.receive(raw), analysers: true });
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  media.setInput(true);
  const track = e.tracks[0]!;
  track.muted = true;
  track.listeners.get("mute")?.();
  const muted = media
    .getDiagnostics()
    .find((row) => row.stage === "input_track");
  assert.deepEqual(
    { ...muted?.counts },
    {
      audio_tracks: 1,
      enabled_tracks: 1,
      live_tracks: 1,
      muted_tracks: 1,
      input_requested: 1,
    },
  );
  assert.equal(
    events.some((event) => event.type === "error"),
    false,
  );
  track.muted = false;
  track.listeners.get("unmute")?.();
  const oldEnded = track.listeners.get("ended")!;
  track.readyState = "ended";
  oldEnded();
  assert.equal(events.filter((event) => event.type === "error").length, 1);
  assert.equal(track.stopped, true);
  assert.equal(track.listeners.size, 0);
  assert.equal(e.peers[0]!.closed, true);
  assert.equal(e.contexts[0].closed, true);
  assert.equal(
    [...e.intervals.values()].some(
      ({ delay }) => delay === 2000 || delay === 50,
    ),
    false,
  );
  const messages = e.messages.length;
  oldEnded();
  e.tick();
  await settle();
  assert.equal(e.messages.length, messages);
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "input_track")?.name,
    "ended",
  );
});

test("OpenAI activity diagnostics expose only bounded audio counters and retain latest snapshots", async () => {
  const media = nativeMedia();
  let bytes = 65536;
  const e = engine({
    receive: (raw) => media.receive(raw),
    analysers: true,
    stats: async () =>
      new Map([
        [
          "private outbound id",
          {
            type: "outbound-rtp",
            kind: "audio",
            bytesSent: bytes,
            packetsSent: 17,
            totalAudioEnergy: 0.25,
            totalSamplesDuration: 3.5,
            audioLevel: 0.4,
            ssrc: "private ssrc",
          },
        ],
        [
          "private inbound id",
          {
            type: "inbound-rtp",
            mediaType: "audio",
            bytesReceived: 2048,
            packetsReceived: 4,
            totalAudioEnergy: 0.125,
            totalSamplesDuration: 1.25,
          },
        ],
        [
          "private source id",
          {
            type: "media-source",
            kind: "audio",
            totalAudioEnergy: 0.5,
            totalSamplesDuration: 4,
            audioLevel: 0.75,
            trackIdentifier: "private track",
          },
        ],
        [
          "private video",
          { type: "outbound-rtp", kind: "video", bytesSent: 999999 },
        ],
        [
          "private candidate",
          {
            type: "local-candidate",
            kind: "audio",
            address: "192.0.2.75",
            url: "private url",
            bytesSent: 123,
          },
        ],
      ]),
  });
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  e.peers[0]!.ontrack?.({ track: {}, streams: [e.stream] });
  await settle();
  const outbound = () =>
    media.getDiagnostics().find((row) => row.stage === "openai_rtp_outbound");
  assert.deepEqual(
    { ...outbound()?.counts },
    {
      audio_reports: 1,
      bytes: 65536,
      packets: 17,
      total_audio_energy: 0.25,
      total_samples_duration: 3.5,
      audio_level: 0.4,
      sample: 1,
    },
  );
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_rtp_inbound")
      ?.counts?.bytes,
    2048,
  );
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_audio_source")
      ?.counts?.audio_level,
    0.75,
  );
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_peer")?.name,
    "connected",
  );
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_ice")?.name,
    "connected",
  );
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_datachannel")
      ?.name,
    "open",
  );
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_context")?.name,
    "running",
  );
  for (let i = 0; i < 40; i++) {
    bytes++;
    e.tickInterval(2000);
    await settle();
  }
  assert.equal(outbound()?.counts?.bytes, 65576);
  assert.equal(outbound()?.counts?.sample, 41);
  assert.equal(
    media.getDiagnostics().filter((row) => row.stage === "openai_rtp_outbound")
      .length,
    1,
  );
  assert.equal(
    media
      .getDiagnostics()
      .some((row) => row.stage === "openai_play" && row.name === "ready"),
    true,
  );
  assert.ok(media.getDiagnostics().length <= 32);
  const copied = outbound()!;
  copied.counts!.bytes = 0;
  assert.equal(outbound()?.counts?.bytes, 65576);
  assert.doesNotMatch(
    JSON.stringify(media.getDiagnostics()),
    /private|192\.0\.2|ssrc|candidate|trackIdentifier|999999/,
  );
  media.close();
});

test("diagnostic numeric fields reject malformed values and getStats errors stay nonfatal", async () => {
  const media = nativeMedia();
  let rejected = false;
  const e = engine({
    receive: (raw) => media.receive(raw),
    stats: async () => {
      if (rejected)
        throw Object.assign(new Error("private stats error"), {
          name: "private exception",
        });
      return new Map([
        [
          "private id",
          {
            type: "outbound-rtp",
            kind: "audio",
            bytesSent: -1,
            packetsSent: 2.5,
            totalAudioEnergy: Infinity,
            totalSamplesDuration: NaN,
            audioLevel: 2,
          },
        ],
      ]);
    },
  });
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  assert.deepEqual(
    {
      ...media
        .getDiagnostics()
        .find((row) => row.stage === "openai_rtp_outbound")?.counts,
    },
    { audio_reports: 1, sample: 1 },
  );
  media.receive(
    JSON.stringify({
      type: "diagnostic",
      generation: 1,
      stage: "openai_playback",
      name: "ready",
      counts: {
        volume: 2,
        paused: 0.5,
        audio_tracks: 17,
        bytes: Number.MAX_SAFE_INTEGER + 1,
        packets: -1,
        total_audio_energy: 1e10,
        audio_level: "private",
        current_time_ms: 1830001,
        ready_state: 5,
        private: 42,
        constructor: 20,
        input_requested: 1,
      },
    }),
  );
  assert.deepEqual(
    {
      ...media.getDiagnostics().find((row) => row.stage === "openai_playback")
        ?.counts,
    },
    { input_requested: 1 },
  );
  rejected = true;
  e.tickInterval(2000);
  await settle();
  assert.equal(
    media.getDiagnostics().find((row) => row.stage === "openai_stats")?.name,
    "Error",
  );
  assert.equal(
    events.some((event) => event.type === "error"),
    false,
  );
  assert.equal(e.tracks[0]!.stopped, false);
  assert.doesNotMatch(JSON.stringify(e.messages), /private/);
  media.close();
});

test("OpenAI stats never overlap and pending stats or play callbacks cannot enter a later generation", async () => {
  const oldStats = deferred<Map<string, Record<string, unknown>>>();
  const oldPlay = deferred<void>();
  const nextTrack = {
    enabled: true,
    readyState: "live",
    muted: false,
    stop() {},
  };
  const nextStream = {
    getTracks: () => [nextTrack],
    getAudioTracks: () => [nextTrack],
  };
  let mediaCalls = 0;
  let statsCalls = 0;
  const e = engine({
    media: async () => (++mediaCalls === 1 ? e.stream : nextStream),
    play: () => oldPlay.promise,
    stats: async () =>
      ++statsCalls === 1 ? await oldStats.promise : new Map(),
  });
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  e.peers[0]!.ontrack?.({ track: {}, streams: [e.stream] });
  const oldEnded = e.tracks[0]!.listeners.get("ended")!;
  e.tickInterval(2000);
  e.tickInterval(2000);
  assert.equal(e.statsRequests(), 1);
  e.command({ type: "close", generation: 1 });
  assert.equal(e.tracks[0]!.listeners.size, 0);
  assert.equal(
    [...e.intervals.values()].some(({ delay }) => delay === 2000),
    false,
  );
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 2,
    requestId: 2,
  });
  await settle();
  assert.equal(e.statsRequests(), 2);
  const before = e.messages.length;
  oldEnded();
  oldStats.resolve(
    new Map([
      ["private", { type: "outbound-rtp", kind: "audio", bytesSent: 7654321 }],
    ]),
  );
  oldPlay.reject(new DOMException("private rejection", "NotAllowedError"));
  await settle();
  assert.equal(e.messages.length, before);
  assert.equal(e.peers[1]!.closed, false);
  assert.equal(nextTrack.enabled, false);
  e.command({ type: "close", generation: 2 });
});

test("the WebView is inaccessible, blocks later navigation, and closes on unmount", () => {
  type Element = { props: Record<string, any> };
  const harness = hookHarness<Element>({
    "react-native": { StyleSheet: { create: (value: unknown) => value } },
    "react-native-webview": { WebView: "WebView" },
    "./voiceMediaDocument": documentModule,
  });
  const module = harness.load(
    fileURLToPath(new URL("./VoiceMediaBridge.tsx", import.meta.url)),
  ) as {
    NativeVoiceMedia: new () => Media;
    VoiceMediaBridge(props: { media: Media; origin: string }): Element;
  };
  const media = new module.NativeVoiceMedia();
  const bridge = harness.mount(() =>
    module.VoiceMediaBridge({ media, origin: "https://app.example.test" }),
  );
  assert.equal(bridge.props.pointerEvents, "none");
  assert.equal(bridge.props.accessibilityElementsHidden, true);
  assert.equal(
    bridge.props.mediaCapturePermissionGrantType,
    "grantIfSameHostElseDeny",
  );
  assert.equal(
    bridge.props.onShouldStartLoadWithRequest({ url: "https://evil.example" }),
    false,
  );
  assert.equal(
    bridge.props.onShouldStartLoadWithRequest({
      url: "https://app.example.test",
    }),
    true,
  );
  bridge.props.onLoadEnd();
  assert.equal(
    bridge.props.onShouldStartLoadWithRequest({
      url: "https://app.example.test",
    }),
    false,
  );
  let closed = 0;
  const originalClose = media.close.bind(media);
  media.close = () => {
    closed++;
    originalClose();
  };
  harness.unmount();
  assert.equal(closed, 1);
});

const levels = (messages: Record<string, any>[]) =>
  messages
    .filter((m) => m.type === "level")
    .map(({ input, output }) => ({ input, output }));

test("OpenAI meters the microphone and remote audio in a silent context", async () => {
  const e = engine({ analysers: true });
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.contexts.length, 1);
  const [input] = e.analysers;
  assert.equal(input.inputs[0].stream, e.stream);
  e.peers[0]!.ontrack?.({ track: {}, streams: [e.stream] });
  const output = e.analysers[1];
  assert.ok(output);
  // Analysers are taps only: nothing is routed to any destination.
  for (const node of e.analysers) assert.deepEqual(node.connections, []);
  input.amplitude = 0.1; // -20 dBFS
  output.amplitude = 0.01; // -40 dBFS
  e.tick();
  assert.deepEqual(levels(e.messages), [{ input: 0.75, output: 0.25 }]);
  e.tick();
  assert.equal(levels(e.messages).length, 1, "unchanged levels are not resent");
  input.amplitude = 0;
  e.tick();
  assert.deepEqual(levels(e.messages).at(-1), { input: 0, output: 0.25 });
  assert.ok(
    e.messages
      .filter((m) => m.type === "level")
      .every((m) => m.generation === 1),
  );
  e.command({ type: "close", generation: 1 });
  assert.equal(e.contexts[0]!.closed, true);
  assert.ok(e.analysers.every((node) => node.disconnected));
  output.amplitude = 1;
  e.tick();
  assert.equal(levels(e.messages).length, 2);
});

test("Grok meters its microphone source and playback worklet without a second output", async () => {
  const e = engine({ analysers: true });
  e.command({
    type: "prepare",
    protocol: "xai_realtime",
    generation: 2,
    requestId: 1,
  });
  await settle();
  assert.equal(e.contexts.length, 1);
  const worklet = e.worklets[0]!;
  assert.equal(worklet.connections.includes(e.contexts[0]!.destination), false);
  const [input, output] = e.analysers;
  assert.ok(worklet.connections.includes(output));
  assert.equal(input.inputs[0].stream, e.stream);
  output.amplitude = 1;
  e.tick();
  assert.deepEqual(levels(e.messages), [{ input: 0, output: 1 }]);
});

test("a runtime without analysers still starts calls, with no loudness events", async () => {
  const e = engine();
  e.command({
    type: "prepare",
    protocol: "openai_live",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.match(e.messages.find((m) => m.type === "response")?.result, /^v=0/);
  assert.equal(
    e.messages.some((m) => m.type === "error"),
    false,
  );
  e.tick();
  assert.deepEqual(levels(e.messages), []);
});

test("the native adapter forwards bounded loudness and drops malformed meters", async () => {
  const media = nativeMedia();
  const events: VoiceMediaEvent[] = [];
  media.subscribe((event) => events.push(event));
  const e = engine({ analysers: true, receive: (raw) => media.receive(raw) });
  media.connect(e.command);
  await media.prepare("openai_live", new AbortController().signal);
  e.analysers[0].amplitude = 0.1;
  e.tick();
  assert.deepEqual(
    JSON.parse(
      JSON.stringify(events.filter((event) => event.type === "level")),
    ),
    [{ type: "level", input: 0.75, output: 0 }],
  );
  for (const bad of [
    { input: 2, output: 0 },
    { input: -0.1, output: 0 },
    { input: "0.5", output: 0 },
    { input: 0.5 },
  ])
    media.receive(JSON.stringify({ type: "level", generation: 1, ...bad }));
  assert.equal(events.filter((event) => event.type === "level").length, 1);
  assert.equal(
    events.some((event) => event.type === "error"),
    false,
  );
  media.close();
});
