import assert from "node:assert/strict";
import test from "node:test";
import {
  NativeOpenAIMedia,
  type NativeAudioChannel,
  type NativeAudioPeer,
  type NativeAudioStream,
  type NativeAudioTrack,
} from "./NativeOpenAIMedia";
import type { VoiceMediaEvent } from "./AssistantVoiceClient";

type Event = { data?: unknown; track?: NativeAudioTrack };
type Listener = (event: Event) => void;

class Events {
  listeners = new Map<string, Set<Listener>>();
  addEventListener(type: string, listener: Listener) {
    const listeners = this.listeners.get(type) ?? new Set();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }
  removeEventListener(type: string, listener: Listener) {
    this.listeners.get(type)?.delete(listener);
  }
  dispatch(type: string, event: Event = {}) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

class Track extends Events implements NativeAudioTrack {
  kind = "audio";
  enabled = true;
  readyState = "live";
  stops = 0;
  volumes: number[] = [];
  stop() {
    this.stops++;
    this.enabled = false;
    this.readyState = "ended";
  }
  _setVolume(volume: number) {
    this.volumes.push(volume);
  }
}

class Stream implements NativeAudioStream {
  track = new Track();
  releases: boolean[] = [];
  getTracks() {
    return [this.track];
  }
  getAudioTracks() {
    return [this.track];
  }
  release(value = true) {
    this.releases.push(value);
  }
}

class Channel extends Events implements NativeAudioChannel {
  closes = 0;
  close() {
    this.closes++;
    this.dispatch("close");
  }
}

class Peer extends Events implements NativeAudioPeer {
  connectionState = "new";
  iceConnectionState = "new";
  iceGatheringState = "complete";
  localDescription: { type: string; sdp: string } | null = null;
  offer = { type: "offer" as const, sdp: "private-sdp-offer" };
  channel = new Channel();
  added: NativeAudioTrack[] = [];
  answers: string[] = [];
  closes = 0;
  statsCalls = 0;
  stats = new Map<string, Record<string, unknown>>();
  statsResult?: Promise<Map<string, Record<string, unknown>>>;
  answerResult?: Promise<void>;
  offerResult?: Promise<{ type: "offer"; sdp: string }>;
  addTrack(track: NativeAudioTrack) {
    this.added.push(track);
  }
  async createOffer() {
    return this.offerResult ?? this.offer;
  }
  async setLocalDescription(description: { type: "offer"; sdp: string }) {
    this.localDescription = description;
  }
  async setRemoteDescription(description: { type: "answer"; sdp: string }) {
    this.answers.push(description.sdp);
    await this.answerResult;
  }
  createDataChannel(label: string) {
    assert.equal(label, "oai-events");
    return this.channel;
  }
  async getStats() {
    this.statsCalls++;
    return this.statsResult ?? this.stats;
  }
  close() {
    this.closes++;
    this.connectionState = "closed";
    this.dispatch("connectionstatechange");
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

function harness() {
  const streams: Stream[] = [];
  const peers: Peer[] = [];
  const events: VoiceMediaEvent[] = [];
  const timeouts = new Map<number, { callback: () => void; ms: number }>();
  const intervals = new Map<number, () => void>();
  let sequence = 0;
  let capture: Promise<NativeAudioStream> | undefined;
  let nextPeer: Peer | undefined;
  let captures = 0;
  const media = new NativeOpenAIMedia({
    getUserMedia: () => {
      captures++;
      if (capture) return capture;
      const stream = new Stream();
      streams.push(stream);
      return Promise.resolve(stream);
    },
    createPeer: () => {
      const peer = nextPeer ?? new Peer();
      nextPeer = undefined;
      peers.push(peer);
      return peer;
    },
    setTimeout: ((callback: () => void, ms: number) => {
      const id = ++sequence;
      timeouts.set(id, { callback, ms });
      return id;
    }) as unknown as typeof setTimeout,
    clearTimeout: ((id: number) => {
      timeouts.delete(id);
    }) as unknown as typeof clearTimeout,
    setInterval: ((callback: () => void) => {
      const id = ++sequence;
      intervals.set(id, callback);
      return id;
    }) as unknown as typeof setInterval,
    clearInterval: ((id: number) => {
      intervals.delete(id);
    }) as unknown as typeof clearInterval,
  });
  media.subscribe((event) => events.push(event));
  const signal = new AbortController();
  return {
    media,
    streams,
    peers,
    events,
    timeouts,
    intervals,
    signal,
    capture(value: Promise<NativeAudioStream>) {
      capture = value;
    },
    peer(value: Peer) {
      nextPeer = value;
    },
    captures: () => captures,
    start: () => media.prepare("openai_live", signal.signal),
    timeout(ms: number) {
      const timer = [...timeouts.values()].find((value) => value.ms === ms);
      assert.ok(timer, `No ${ms} ms timeout`);
      timer.callback();
    },
    async sample() {
      for (const callback of intervals.values()) callback();
      await tick();
    },
    errors: () => events.filter((event) => event.type === "error"),
  };
}

test("native preparation keeps capture disabled, negotiates once and releases real native resources", async () => {
  const h = harness();
  try {
    assert.equal(await h.start(), "private-sdp-offer");
    assert.equal(h.captures(), 1);
    assert.equal(h.streams[0].track.enabled, false);
    assert.equal(h.peers[0].added[0], h.streams[0].track);
    assert.equal(h.intervals.size, 1);
    await h.media.answer("private-sdp-answer");
    assert.deepEqual(h.peers[0].answers, ["private-sdp-answer"]);
    assert.equal(
      JSON.stringify(h.media.getDiagnostics()).includes("private-sdp"),
      false,
    );
    h.media.close();
    assert.deepEqual(h.streams[0].releases, [true]);
    assert.equal(h.streams[0].track.stops, 1);
    assert.equal(h.peers[0].closes, 1);
    assert.equal(h.peers[0].channel.closes, 1);
    assert.equal(h.intervals.size, 0);
    assert.equal(h.timeouts.size, 0);
    assert.equal(h.errors().length, 0);
  } finally {
    h.media.close();
  }
});

test("readiness requires exactly the bounded session.started event and fires once", async () => {
  const h = harness();
  try {
    await h.start();
    const channel = h.peers[0].channel;
    channel.dispatch("open");
    for (const data of [
      "{",
      {},
      JSON.stringify({ type: "session.created" }),
      "x".repeat(262145),
    ])
      channel.dispatch("message", { data });
    assert.equal(
      h.events.filter((event) => event.type === "provider-ready").length,
      0,
    );
    channel.dispatch("message", {
      data: JSON.stringify({
        type: "session.started",
        private: "not-retained",
      }),
    });
    channel.dispatch("message", {
      data: JSON.stringify({ type: "session.started" }),
    });
    assert.equal(
      h.events.filter((event) => event.type === "provider-ready").length,
      1,
    );
    assert.equal(
      JSON.stringify(h.media.getDiagnostics()).includes("not-retained"),
      false,
    );
  } finally {
    h.media.close();
  }
});

test("native playback supports hold attenuation, speaker mute and late remote tracks", async () => {
  const h = harness();
  try {
    await h.start();
    const remote = new Track();
    h.media.hold(true);
    h.peers[0].dispatch("track", { track: remote });
    assert.equal(remote.enabled, true);
    assert.deepEqual(remote.volumes, [0.25]);
    h.media.muteSpeaker(true);
    assert.equal(remote.enabled, false);
    assert.equal(remote.volumes.at(-1), 0);
    const another = new Track();
    h.peers[0].dispatch("track", { track: another });
    assert.equal(another.enabled, false);
    h.media.muteSpeaker(false);
    assert.equal(another.volumes.at(-1), 0.25);
    h.media.hold(false);
    await h.media.resume();
    assert.equal(remote.volumes.at(-1), 1);
    h.media.close();
    assert.equal(remote.enabled, false);
    assert.equal(remote.stops, 1);
  } finally {
    h.media.close();
  }
});

test("input toggles only the live input track and never enables it from hold", async () => {
  const h = harness();
  try {
    await h.start();
    h.media.hold(true);
    assert.equal(h.streams[0].track.enabled, false);
    h.media.setInput(true);
    assert.equal(h.streams[0].track.enabled, true);
    h.media.setInput(false);
    assert.equal(h.streams[0].track.enabled, false);
  } finally {
    h.media.close();
  }
});

test("abort during getUserMedia rejects promptly and frees a late native capture", async () => {
  const h = harness();
  const pending = deferred<NativeAudioStream>();
  h.capture(pending.promise);
  const start = h.start();
  h.signal.abort();
  await assert.rejects(start, /closed/);
  const stream = new Stream();
  pending.resolve(stream);
  await tick();
  assert.deepEqual(stream.releases, [true]);
  assert.equal(stream.track.enabled, false);
  assert.equal(h.peers.length, 0);
  assert.equal(h.timeouts.size, 0);
  assert.equal(h.errors().length, 0);
});

test("a native capture timeout frees a late stream and releases microphone ownership", async () => {
  const h = harness();
  const pending = deferred<NativeAudioStream>();
  h.capture(pending.promise);
  const start = h.start();
  h.timeout(20000);
  await assert.rejects(start, /too long/);
  assert.equal(h.errors().length, 1);
  const next = harness();
  try {
    await next.start();
  } finally {
    next.media.close();
  }
  const stream = new Stream();
  pending.resolve(stream);
  await tick();
  assert.deepEqual(stream.releases, [true]);
});

test("only one adapter may own native input and closing one cannot close another", async () => {
  const first = harness(),
    second = harness();
  try {
    await first.start();
    await assert.rejects(second.start(), /already in use/);
    assert.equal(second.captures(), 0);
    first.media.close();
    await second.start();
    first.media.close();
    assert.equal(second.streams[0].track.readyState, "live");
  } finally {
    first.media.close();
    second.media.close();
  }
});

test("ICE gathering must complete before returning the actual complete local offer", async () => {
  const h = harness();
  const peer = new Peer();
  peer.iceGatheringState = "gathering";
  h.peer(peer);
  const start = h.start();
  try {
    await tick();
    assert.equal(h.timeouts.size, 1);
    peer.localDescription!.sdp = "complete-offer";
    peer.iceGatheringState = "complete";
    peer.dispatch("icegatheringstatechange");
    assert.equal(await start, "complete-offer");
    assert.equal(peer.listeners.get("icegatheringstatechange")?.size, 0);
  } finally {
    h.media.close();
  }
});

test("ICE timeout rejects preparation and tears down input, peer and temporary listeners", async () => {
  const h = harness();
  const peer = new Peer();
  peer.iceGatheringState = "gathering";
  h.peer(peer);
  const start = h.start();
  await tick();
  h.timeout(5000);
  await assert.rejects(start, /too long/);
  assert.deepEqual(h.streams[0].releases, [true]);
  assert.equal(peer.closes, 1);
  assert.equal(peer.listeners.get("icegatheringstatechange")?.size, 0);
  assert.equal(h.errors().length, 1);
});

test("an abort during answer cannot leave local audio or a pending operation alive", async () => {
  const h = harness();
  const pending = deferred<void>();
  await h.start();
  h.peers[0].answerResult = pending.promise;
  const answer = h.media.answer("answer");
  h.signal.abort();
  await assert.rejects(answer, /closed/);
  pending.resolve();
  await tick();
  assert.deepEqual(h.streams[0].releases, [true]);
  assert.equal(h.timeouts.size, 0);
  assert.equal(h.errors().length, 0);
});

test("offer and answer operations time out without retaining capture or callbacks", async () => {
  for (const operation of ["offer", "answer"]) {
    const h = harness();
    const peer = new Peer();
    h.peer(peer);
    let task: Promise<unknown>;
    if (operation === "offer") {
      peer.offerResult = deferred<{ type: "offer"; sdp: string }>().promise;
      task = h.start();
      await tick();
    } else {
      await h.start();
      peer.answerResult = deferred<void>().promise;
      task = h.media.answer("answer");
    }
    h.timeout(15000);
    await assert.rejects(task, /too long/);
    assert.deepEqual(h.streams[0].releases, [true]);
    assert.equal(peer.closes, 1);
    assert.equal(h.errors().length, 1);
    assert.equal(h.timeouts.size, 0);
    assert.equal(h.intervals.size, 0);
  }
});

test("an ended microphone and failed peer each terminate media exactly once", async () => {
  for (const failure of ["track", "peer", "channel"]) {
    const h = harness();
    try {
      await h.start();
      if (failure === "track") h.streams[0].track.dispatch("ended");
      if (failure === "peer") {
        h.peers[0].connectionState = "failed";
        h.peers[0].dispatch("connectionstatechange");
      }
      if (failure === "channel") h.peers[0].channel.dispatch("close");
      assert.equal(h.errors().length, 1);
      assert.deepEqual(h.streams[0].releases, [true]);
      assert.equal(h.peers[0].closes, 1);
      h.peers[0].channel.dispatch("error");
      assert.equal(h.errors().length, 1);
    } finally {
      h.media.close();
    }
  }
});

test("stale native callbacks cannot end or announce readiness for the next capture", async () => {
  const h = harness();
  try {
    await h.start();
    const staleTrack = [...h.streams[0].track.listeners.get("ended")!][0];
    const staleMessage = [...h.peers[0].channel.listeners.get("message")!][0];
    h.media.close();
    await h.start();
    staleTrack({});
    staleMessage({ data: JSON.stringify({ type: "session.started" }) });
    assert.equal(h.streams[1].track.readyState, "live");
    assert.equal(h.errors().length, 0);
    assert.equal(
      h.events.some((event) => event.type === "provider-ready"),
      false,
    );
  } finally {
    h.media.close();
  }
});

test("a remote track already ended on arrival fails rather than reporting usable playback", async () => {
  const h = harness();
  try {
    await h.start();
    const remote = new Track();
    remote.readyState = "ended";
    h.peers[0].dispatch("track", { track: remote });
    assert.equal(h.errors().length, 1);
    assert.equal(h.media.playback().playback_audible, false);
    assert.deepEqual(h.streams[0].releases, [true]);
  } finally {
    h.media.close();
  }
});

test("bounded stats emit sanitized levels and RTP counters without copying provider data", async () => {
  const h = harness();
  try {
    await h.start();
    const peer = h.peers[0],
      remote = new Track();
    peer.dispatch("track", { track: remote });
    peer.connectionState = "connected";
    peer.stats = new Map([
      [
        "mic",
        {
          type: "media-source",
          kind: "audio",
          audioLevel: 0.5,
          label: "secret-label",
        },
      ],
      ["sent", { type: "outbound-rtp", kind: "audio", bytesSent: 321 }],
      [
        "received",
        {
          type: "inbound-rtp",
          kind: "audio",
          audioLevel: 1.5,
          bytesReceived: 654,
          jitterBufferEmittedCount: 48000,
          codecId: "codec",
          private: "private-provider-data",
        },
      ],
      ["codec", { clockRate: 48000 }],
    ]);
    h.media.setInput(true);
    await h.sample();
    assert.deepEqual(h.events.at(-1), { type: "level", input: 0.5, output: 1 });
    assert.deepEqual(h.media.playback(), {
      playback_ms: 1000,
      playback_audible: true,
    });
    const diagnostics = JSON.stringify(h.media.getDiagnostics());
    assert.equal(diagnostics.includes("secret-label"), false);
    assert.equal(diagnostics.includes("private-provider-data"), false);
    assert.equal(diagnostics.includes("654"), true);
    h.media.muteSpeaker(true);
    h.media.setInput(false);
    await h.sample();
    assert.deepEqual(h.events.at(-1), { type: "level", input: 0, output: 0 });
    assert.equal(h.media.playback().playback_audible, false);
  } finally {
    h.media.close();
  }
});

test("overlapping or stale stats never produce cross-generation levels", async () => {
  const h = harness();
  const pending = deferred<Map<string, Record<string, unknown>>>();
  try {
    await h.start();
    h.peers[0].statsResult = pending.promise;
    await h.sample();
    await h.sample();
    assert.equal(h.peers[0].statsCalls, 1);
    h.media.close();
    await h.start();
    const count = h.events.length;
    pending.resolve(
      new Map([
        ["audio", { type: "media-source", kind: "audio", audioLevel: 1 }],
      ]),
    );
    await tick();
    assert.equal(h.events.length, count);
  } finally {
    h.media.close();
  }
});

test("capture rejection, malformed offer and unsupported protocol never leak raw errors", async () => {
  const h = harness();
  try {
    await assert.rejects(
      h.media.prepare("xai_realtime", h.signal.signal),
      /not supported/,
    );
    assert.equal(h.captures(), 0);
    h.capture(Promise.reject(new Error("private-device-name")));
    await assert.rejects(
      h.start(),
      (error: Error) => !error.message.includes("private-device-name"),
    );
  } finally {
    h.media.close();
  }
  const invalid = harness();
  const peer = new Peer();
  peer.offer.sdp = " ";
  invalid.peer(peer);
  await assert.rejects(invalid.start(), /could not be started/);
  assert.deepEqual(invalid.streams[0].releases, [true]);
});
