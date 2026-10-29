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
    media?: Promise<unknown>;
    receive?: (raw: string) => void;
    iceComplete?: boolean;
    loopbackConnected?: boolean;
    candidateSDP?: string;
  } = {},
) {
  const messages: Record<string, any>[] = [];
  const peers: Peer[] = [];
  const tracks = [
    {
      enabled: true,
      stopped: false,
      stop() {
        this.stopped = true;
      },
    },
  ];
  const stream = { getTracks: () => tracks, getAudioTracks: () => tracks };
  const timers = new Map<number, () => void>();
  const intervals: (() => void)[] = [];
  const documentListeners = new Map<string, () => void>();
  let timerId = 0;
  let micRequests = 0;
  const audio = {
    autoplay: false,
    currentTime: 0,
    paused: true,
    muted: false,
    volume: 1,
    srcObject: null as unknown,
    removed: false,
    setAttribute() {},
    async play() {
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
    closed = false;
    addedCandidates = 0;
    channel = { label: "", onmessage: (_: { data: string }) => {} };
    ontrack?: (event: unknown) => void;
    listeners = new Map<string, () => void>();
    constructor() {
      peers.push(this);
    }
    addTrack() {}
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
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
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
    connect(value: unknown) {
      this.connections.push(value);
    }
    disconnect() {}
  }
  const sandbox: any = {
    isSecureContext: options.secure !== false,
    navigator: {
      mediaDevices: {
        getUserMedia: async () => {
          micRequests++;
          return options.media ? await options.media : stream;
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
    setInterval: (callback: () => void) => intervals.push(callback),
    clearInterval() {},
    addEventListener() {},
    __nyxVoiceWorklet: VOICE_WORKLET_SOURCE,
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
    timers,
    tick: () => intervals.forEach((callback) => callback()),
    micRequests: () => micRequests,
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
    .filter((row) => row.counts && row.stage !== "runtime");
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

test("dictation captures PCM24k without a conversational peer or microphone playback", async () => {
  const e = engine();
  e.command({
    type: "prepare",
    protocol: "dictation",
    generation: 1,
    requestId: 1,
  });
  await settle();
  assert.equal(e.peers.length, 0);
  assert.equal(e.worklets.length, 1);
  assert.equal(e.tracks[0]!.enabled, false);
  e.command({ type: "input", enabled: true, generation: 1 });
  assert.equal(e.tracks[0]!.enabled, true);
  const worklet = e.worklets[0];
  worklet.port.onmessage({
    data: { type: "pcm", bytes: new ArrayBuffer(960), generation: 1 },
  });
  assert.equal(e.messages.filter((m) => m.type === "pcm").length, 1);
  e.command({ type: "close", generation: 1 });
  assert.equal(e.tracks[0]!.stopped, true);
  assert.equal(e.contexts[0]!.closed, true);
  worklet.port.onmessage({
    data: { type: "pcm", bytes: new ArrayBuffer(960), generation: 1 },
  });
  assert.equal(e.messages.filter((m) => m.type === "pcm").length, 1);
});
