import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../../lib/api/ApiError";
import {
  AssistantVoiceClient,
  type AssistantVoiceApi,
  type VoiceClientClock,
  type VoiceControlSocket,
  type VoiceMediaAdapter,
  type VoiceMediaEvent,
} from "./AssistantVoiceClient";
import type {
  VoiceControlRequest,
  VoicePreferences,
  VoiceSession,
  VoiceSnapshot,
  VoiceStartRequest,
} from "./voiceContracts";
import type { VoiceStartRecovery } from "./voiceStartError";

const conversationId = "nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const sessionId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const requestId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const preferences: VoicePreferences = {
  service_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  voice: "alloy",
  input_mode: "push_to_talk",
  language: null,
  notify_on_completion: true,
};

function session(changes: Partial<VoiceSession> = {}): VoiceSession {
  return {
    id: sessionId,
    created_at: "2026-10-16T00:00:00Z",
    closed_at: null,
    generation: 1,
    state: "active",
    muted: false,
    input_muted: false,
    end_requested: false,
    control_revision: 0,
    observed_seconds: 0,
    reserved_until: 15,
    final_usage_confirmed: false,
    end_reason: null,
    idle_warning: false,
    resumable: true,
    ...changes,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

class Media implements VoiceMediaAdapter {
  inputs: boolean[] = [];
  holds: boolean[] = [];
  speaker: boolean[] = [];
  answers: string[] = [];
  enqueued: { bytes: ArrayBuffer; endMs: number }[] = [];
  listeners = new Set<(event: VoiceMediaEvent) => void>();
  prepared: string[] = [];
  closed = 0;
  flushed = 0;
  resumed = 0;
  report = { playback_ms: 123, playback_audible: true };
  prepareResult: Promise<string> = Promise.resolve("offer");
  answerResult?: Promise<void>;
  resumeResult = Promise.resolve();
  async prepare(protocol: string) {
    this.prepared.push(protocol);
    return this.prepareResult;
  }
  async answer(sdp: string) {
    this.answers.push(sdp);
    return this.answerResult;
  }
  setInput(enabled: boolean) {
    this.inputs.push(enabled);
  }
  hold(pressed: boolean) {
    this.holds.push(pressed);
  }
  muteSpeaker(muted: boolean) {
    this.speaker.push(muted);
  }
  async resume() {
    this.resumed++;
    return this.resumeResult;
  }
  enqueue(bytes: ArrayBuffer, endMs: number) {
    this.enqueued.push({ bytes, endMs });
  }
  flush() {
    this.flushed++;
  }
  playback() {
    return this.report;
  }
  close() {
    this.closed++;
  }
  subscribe(listener: (event: VoiceMediaEvent) => void) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  emit(event: VoiceMediaEvent) {
    this.listeners.forEach((listener) => listener(event));
  }
  get input() {
    return this.inputs.at(-1);
  }
}

class Socket implements VoiceControlSocket {
  readyState = 1;
  bufferedAmount = 0;
  binaryType = "";
  onmessage: VoiceControlSocket["onmessage"] = null;
  onclose: VoiceControlSocket["onclose"] = null;
  onerror: VoiceControlSocket["onerror"] = null;
  sent: (string | ArrayBuffer)[] = [];
  closed = 0;
  send(data: string | ArrayBuffer) {
    this.sent.push(data);
  }
  close() {
    this.closed++;
    this.readyState = 3;
  }
  receive(data: unknown) {
    this.onmessage?.({
      data:
        typeof data === "object" && !(data instanceof ArrayBuffer)
          ? JSON.stringify(data)
          : data,
    });
  }
  events() {
    return this.sent
      .filter((value): value is string => typeof value === "string")
      .map((value) => JSON.parse(value));
  }
}

class Clock implements VoiceClientClock {
  intervals = new Map<number, () => void>();
  timeouts = new Map<number, () => void>();
  sequence = 0;
  setInterval(callback: () => void) {
    const id = ++this.sequence;
    this.intervals.set(id, callback);
    return id;
  }
  clearInterval(handle: unknown) {
    this.intervals.delete(handle as number);
  }
  setTimeout(callback: () => void) {
    const id = ++this.sequence;
    this.timeouts.set(id, callback);
    return id;
  }
  clearTimeout(handle: unknown) {
    this.timeouts.delete(handle as number);
  }
  tick() {
    [...this.intervals.values()].forEach((callback) => callback());
  }
  expire() {
    [...this.timeouts.values()].forEach((callback) => callback());
  }
}

function setup() {
  const media = new Media();
  const socket = new Socket();
  const clock = new Clock();
  const starts: {
    thread: string;
    input: VoiceStartRequest;
    signal?: AbortSignal;
  }[] = [];
  const controls: {
    thread: string;
    id: string;
    input: VoiceControlRequest;
    signal?: AbortSignal;
  }[] = [];
  const cancels: { thread: string; id: string }[] = [];
  const errors: string[] = [];
  const recoveries: (VoiceStartRecovery | undefined)[] = [];
  const conflicts: (VoiceSession | null)[] = [];
  const snapshots: VoiceSnapshot[] = [];
  const paths: string[] = [];
  let closed = 0;
  let activeCalls = 0;
  let nextId = 0;
  const responses = {
    start: async (): Promise<unknown> => ({
      session: session(),
      sdp_answer: "answer",
    }),
    active: async (): Promise<unknown> => ({ session: null }),
    control: async (input: VoiceControlRequest): Promise<unknown> =>
      session({ muted: input.action === "mute", control_revision: 1 }),
    socket: async (): Promise<VoiceControlSocket> => socket,
    cancel: async (): Promise<void> => {},
  };
  const api: AssistantVoiceApi = {
    start: async (thread, input, signal) => {
      starts.push({ thread, input, signal });
      return responses.start();
    },
    active: async () => {
      activeCalls++;
      return responses.active();
    },
    control: async (thread, id, input, signal) => {
      controls.push({ thread, id, input, signal });
      return responses.control(input);
    },
    cancelRequest: async (thread, id) => {
      cancels.push({ thread, id });
      return responses.cancel();
    },
  };
  const client = new AssistantVoiceClient({
    conversationId,
    media,
    api,
    clock,
    createSocket: async (path) => {
      paths.push(path);
      return responses.socket();
    },
    onSnapshot: (value) => snapshots.push(value),
    onError: (value, recovery) => {
      errors.push(value);
      recoveries.push(recovery);
    },
    onConflict: (value) => conflicts.push(value),
    onClosed: () => {
      closed++;
    },
    randomId: () =>
      `dddddddd-dddd-4ddd-8ddd-${String(++nextId).padStart(12, "0")}`,
  });
  const snapshot = (changes: Partial<VoiceSession> = {}) =>
    socket.receive({
      type: "snapshot",
      session: session(changes),
      captions: [],
      tasks: [],
    });
  const open = async (
    protocol: "openai_live" | "xai_realtime" = "openai_live",
    inputMode: "push_to_talk" | "automatic" = "push_to_talk",
  ) => {
    await client.start({ ...preferences, input_mode: inputMode }, protocol);
    if (protocol === "openai_live") media.emit({ type: "provider-ready" });
    snapshot();
  };
  return {
    client,
    media,
    socket,
    clock,
    starts,
    controls,
    cancels,
    errors,
    recoveries,
    conflicts,
    snapshots,
    paths,
    responses,
    snapshot,
    open,
    count: () => ({ closed, activeCalls }),
  };
}

test("a call creates exactly once, exchanges SDP and waits for both provider and authoritative readiness", async () => {
  const e = setup();
  await e.client.start(preferences);
  assert.equal(e.media.input, false);
  assert.equal(e.starts.length, 1);
  assert.equal(e.starts[0]!.thread, conversationId);
  assert.equal(e.starts[0]!.input.sdp_offer, "offer");
  assert.deepEqual(e.media.answers, ["answer"]);
  e.snapshot();
  e.client.hold(true);
  assert.equal(e.media.input, false);
  e.media.emit({ type: "provider-ready" });
  assert.equal(e.media.input, false, "hold before readiness must not latch");
  e.client.hold(true);
  assert.equal(e.media.input, true);
  await e.client.start(preferences);
  assert.equal(e.starts.length, 1);
  assert.equal(e.socket.binaryType, "arraybuffer");
  assert.ok(e.paths[0]!.endsWith(`/${sessionId}/stream`));
  await e.client.end();
});

test("automatic mode starts only when provider, authoritative session and input permissions agree", async () => {
  const e = setup();
  await e.client.start({ ...preferences, input_mode: "automatic" });
  e.media.emit({ type: "provider-ready" });
  e.snapshot({ state: "starting" });
  assert.equal(e.media.input, false);
  e.snapshot();
  assert.equal(e.media.input, true);
  e.snapshot({ input_muted: true });
  assert.equal(e.media.input, false);
  e.snapshot({ muted: true });
  assert.equal(e.media.input, false);
  await e.client.end();
});

test("Grok PTT begins before PCM and release commits once while restoring output", async () => {
  const e = setup();
  await e.open("xai_realtime");
  assert.equal(e.starts[0]!.input.sdp_offer, "");
  assert.deepEqual(e.media.answers, []);
  e.media.emit({ type: "pcm", bytes: new ArrayBuffer(960) });
  assert.deepEqual(e.socket.sent, []);
  e.client.hold(true);
  e.client.hold(true);
  const pcm = new ArrayBuffer(960);
  e.media.emit({ type: "pcm", bytes: pcm });
  e.client.hold(false);
  e.client.hold(false);
  e.media.emit({ type: "pcm", bytes: new ArrayBuffer(960) });
  assert.deepEqual(e.socket.events(), [
    { type: "ptt_begin" },
    { type: "ptt_commit" },
  ]);
  assert.equal(e.socket.sent[1], pcm);
  assert.equal(e.media.input, false);
  assert.equal(e.media.holds.at(-1), false);
  await e.client.end();
});

test("Grok automatic mode is refused before opening media or creating a paid call", async () => {
  const e = setup();
  await e.client.start(
    { ...preferences, input_mode: "automatic" },
    "xai_realtime",
  );
  assert.equal(e.starts.length, 0);
  assert.deepEqual(e.media.prepared, []);
  assert.match(e.errors[0]!, /Hold to talk/);
  assert.equal(e.client.isClosed, true);
});

test("media denial cannot create a server session or expose local exception details", async () => {
  const e = setup();
  e.media.prepareResult = Promise.reject(
    new Error("private device identifier"),
  );
  await e.client.start(preferences);
  assert.equal(e.starts.length, 0);
  assert.equal(e.count().activeCalls, 0);
  assert.equal(
    e.errors.some((error) => error.includes("private device")),
    false,
  );
  assert.equal(e.client.isClosed, true);
});

test("a denied microphone is diagnosed before any server session is created", async () => {
  const e = setup();
  const denied = new Error("private microphone identifier");
  denied.name = "NotAllowedError";
  e.media.prepareResult = Promise.reject(denied);
  await e.client.start(preferences);
  assert.deepEqual(e.errors, ["Microphone permission was denied"]);
  assert.equal(e.starts.length, 0);
  assert.equal(e.count().activeCalls, 0);
});

test("HTTP startup rejection reports safe provider diagnostics without retrying creation", async () => {
  const e = setup();
  e.responses.start = async () => {
    throw new ApiError({
      statusCode: 503,
      errorCode: 12501,
      errorKey: "voice_provider_unavailable",
      message: "private upstream body and credential",
      details: {
        stage: "provider_create",
        reason: "private reason",
        provider: "openai",
        provider_status: 400,
        provider_code: "invalid_value",
        provider_param: "session.audio.input.format",
      },
    });
  };
  await e.client.start(preferences);
  await e.client.start(preferences);
  assert.deepEqual(e.errors, [
    "Voice provider rejected the session (OpenAI 400: invalid_value · session.audio.input.format · code 12501)",
  ]);
  assert.equal(e.starts.length, 1);
  assert.equal(e.count().activeCalls, 1);
  assert.deepEqual(e.controls, []);
  assert.equal(e.media.input, false);
  assert.equal(e.client.isClosed, true);
});

test("response, local WebRTC and control transport startup failures retain their distinct safe phase", async () => {
  for (const phase of ["response", "webrtc", "transport"] as const) {
    const e = setup();
    if (phase === "response")
      e.responses.start = async () => ({ session: session(), sdp_answer: 7 });
    else if (phase === "webrtc")
      e.media.answerResult = Promise.reject(new Error("private SDP"));
    else
      e.responses.socket = async () => {
        throw new Error("private websocket URL and auth headers");
      };
    await e.client.start(preferences);
    const expected = {
      response: "Voice server returned an invalid session answer",
      webrtc: "WebRTC negotiation failed",
      transport: "Voice control connection could not open",
    };
    assert.deepEqual(e.errors, [expected[phase]], phase);
    assert.equal(e.starts.length, 1, phase);
    assert.equal(e.count().activeCalls, 0, phase);
    assert.deepEqual(
      e.controls.map((item) => item.input.action),
      ["end"],
      phase,
    );
    assert.equal(e.media.input, false, phase);
    assert.equal(e.client.isClosed, true, phase);
  }
});

test("HTTP billing rejection forwards recovery while disposing media without retrying", async () => {
  const e = setup();
  e.responses.start = async () => {
    throw new ApiError({
      statusCode: 402,
      errorCode: 11300,
      errorKey: "insufficient_credits",
      message: "private wallet state",
      details: { stage: "billing_reservation", reason: "private balance" },
    });
  };
  await e.client.start(preferences);
  await e.client.start(preferences);
  assert.deepEqual(e.errors, [
    "Insufficient voice credits (billing_reservation · code 11300)",
  ]);
  assert.equal(e.recoveries[0]?.kind, "credits");
  assert.match(e.recoveries[0]!.message, /initial voice reservation/);
  assert.equal(e.starts.length, 1);
  assert.equal(e.media.input, false);
  assert.equal(e.media.closed, 1);
  assert.equal(e.client.isClosed, true);
  assert.deepEqual(e.controls, []);
});

test("streamed billing failure forwards recovery and still ends its owned session", async () => {
  const e = setup();
  await e.open();
  e.socket.receive({
    type: "start_failed",
    error: {
      error: "wallet_suspended",
      error_code: 11307,
      message: "private wallet state",
      details: { stage: "billing_reservation", reason: "private reason" },
    },
  });
  assert.equal(e.recoveries[0]?.kind, "wallet_suspended");
  assert.match(e.errors[0]!, /billing_reservation/);
  assert.equal(e.errors[0]!.includes("private"), false);
  assert.equal(e.media.input, false);
  assert.equal(e.client.isClosed, true);
  assert.deepEqual(
    e.controls.map((item) => item.input.action),
    ["end"],
  );
});

test("startup failure from the control stream uses the same sanitized provider diagnosis", async () => {
  const e = setup();
  await e.open();
  e.socket.receive({
    type: "start_failed",
    error: {
      error: "voice_provider_unavailable",
      error_code: 12501,
      message: "private body must never be displayed",
      details: {
        stage: "transport",
        reason: "private reason",
        provider: "xai",
        provider_status: 502,
        provider_type: "server_error",
      },
    },
  });
  assert.deepEqual(e.errors, [
    "Voice transport could not connect (xAI 502: server_error · code 12501)",
  ]);
  assert.deepEqual(
    e.controls.map((item) => item.input.action),
    ["end"],
  );
  assert.equal(e.client.isClosed, true);
});

test("a 409 observes the active call without deleting it or retrying session creation", async () => {
  const e = setup();
  e.responses.start = async () => {
    throw { statusCode: 409 };
  };
  const other = session({ id: requestId });
  e.responses.active = async () => ({ session: other });
  await e.client.start(preferences);
  assert.equal(e.starts.length, 1);
  assert.equal(e.count().activeCalls, 1);
  assert.deepEqual(e.conflicts, [other]);
  assert.deepEqual(e.controls, []);
  assert.match(e.errors[0]!, /Another voice call/);
});

test("a lost POST response is reconciled once through active, never through another POST", async () => {
  const e = setup();
  e.responses.start = async () => {
    throw new Error("network");
  };
  e.responses.active = async () => ({ session: session() });
  await e.client.start(preferences);
  await e.client.start(preferences);
  assert.equal(e.starts.length, 1);
  assert.equal(e.count().activeCalls, 1);
  assert.equal(e.conflicts[0]?.id, sessionId);
  assert.deepEqual(
    e.controls,
    [],
    "unknown ownership cannot end another live session",
  );
});

test("an unverifiable active lookup reports explicit unknown ownership", async () => {
  const e = setup();
  e.responses.start = async () => {
    throw new Error("network");
  };
  e.responses.active = async () => ({ session: { id: "invalid" } });
  await e.client.start(preferences);
  assert.deepEqual(e.conflicts, [null]);
  assert.equal(e.client.isClosed, true);
});

test("ending while microphone setup is pending closes any late device result without POST", async () => {
  const e = setup();
  const waiting = deferred<string>();
  e.media.prepareResult = waiting.promise;
  const starting = e.client.start(preferences);
  await e.client.end();
  waiting.resolve("offer");
  await starting;
  assert.equal(e.starts.length, 0);
  assert.equal(e.media.closed, 2, "late prepare is explicitly cleaned up too");
  assert.equal(e.count().closed, 1);
});

test("ending during POST cleans up a late successful owned session exactly once", async () => {
  const e = setup();
  const waiting = deferred<unknown>();
  e.responses.start = () => waiting.promise;
  const starting = e.client.start(preferences);
  for (let step = 0; step < 8 && !e.starts.length; step++)
    await Promise.resolve();
  assert.equal(e.starts.length, 1);
  await e.client.end();
  assert.equal(e.starts[0]!.signal!.aborted, true);
  waiting.resolve({ session: session(), sdp_answer: "answer" });
  await starting;
  assert.deepEqual(
    e.controls.map((value) => value.input.action),
    ["end"],
  );
  assert.equal(
    e.controls[0]!.signal,
    undefined,
    "durable End survives local abort",
  );
  assert.deepEqual(e.media.answers, []);
  assert.deepEqual(e.paths, []);
  assert.equal(e.count().closed, 1);
});

test("ending while socket setup is pending closes the late socket and ignores its events", async () => {
  const e = setup();
  const waiting = deferred<VoiceControlSocket>();
  e.responses.socket = () => waiting.promise;
  const starting = e.client.start(preferences);
  for (let step = 0; step < 8 && !e.paths.length; step++)
    await Promise.resolve();
  assert.equal(e.paths.length, 1);
  await e.client.end();
  waiting.resolve(e.socket);
  await starting;
  assert.equal(e.socket.closed, 1);
  assert.equal(e.socket.onmessage, null);
  assert.equal(e.count().closed, 1);
});

test("an invalid SDP response with a valid owned session still sends durable End", async () => {
  const e = setup();
  e.responses.start = async () => ({ session: session(), sdp_answer: 7 });
  await e.client.start(preferences);
  assert.deepEqual(
    e.controls.map((value) => value.input.action),
    ["end"],
  );
  assert.equal(e.count().activeCalls, 0);
  assert.deepEqual(e.media.answers, []);
});

test("socket loss immediately disables input, ends once and never reconnects", async () => {
  const e = setup();
  await e.open("xai_realtime");
  e.client.hold(true);
  assert.equal(e.media.input, true);
  const staleMessage = e.socket.onmessage!;
  e.socket.onclose!();
  assert.equal(e.media.input, false);
  assert.equal(e.client.isClosed, true);
  e.clock.tick();
  e.clock.expire();
  staleMessage({
    data: JSON.stringify({
      type: "snapshot",
      session: session(),
      captions: [],
      tasks: [],
    }),
  });
  e.media.emit({ type: "pcm", bytes: new ArrayBuffer(960) });
  assert.equal(e.paths.length, 1);
  assert.equal(e.snapshots.length, 1);
  assert.deepEqual(
    e.controls.map((value) => value.input.action),
    ["end"],
  );
  assert.equal(e.clock.intervals.size, 0);
  assert.equal(e.clock.timeouts.size, 0);
});

test("missing authoritative readiness times out without ever enabling the microphone", async () => {
  const e = setup();
  await e.client.start({ ...preferences, input_mode: "automatic" });
  e.media.emit({ type: "provider-ready" });
  e.clock.expire();
  assert.equal(e.media.inputs.includes(true), false);
  assert.equal(e.client.isClosed, true);
  assert.match(e.errors[0]!, /timed out/);
});

test("an active control stream cannot mask missing provider readiness or leave a paid call stuck", async () => {
  const e = setup();
  await e.client.start({ ...preferences, input_mode: "automatic" });
  e.snapshot();
  assert.equal(e.media.input, false);
  e.clock.expire();
  assert.equal(e.client.isClosed, true);
  assert.deepEqual(
    e.controls.map((value) => value.input.action),
    ["end"],
  );
});

test("a locally closed socket stops microphone frames even before the close callback arrives", async () => {
  const e = setup();
  await e.open("xai_realtime");
  e.client.hold(true);
  e.socket.readyState = 3;
  e.media.emit({ type: "pcm", bytes: new ArrayBuffer(960) });
  assert.equal(e.client.isClosed, true);
  assert.equal(e.media.input, false);
  assert.equal(e.socket.sent.length, 1);
});

test("a durable server end request disables input immediately before terminal settlement", async () => {
  const e = setup();
  await e.open("openai_live", "automatic");
  assert.equal(e.media.input, true);
  e.snapshot({ end_requested: true });
  assert.equal(e.media.input, false);
  e.client.hold(true);
  assert.equal(e.media.input, false);
  await e.client.end();
});

test("heartbeat reports verified playback and monotonically increasing revisions", async () => {
  const e = setup();
  await e.client.start(preferences);
  e.clock.tick();
  assert.deepEqual(e.socket.sent, []);
  e.snapshot();
  e.clock.tick();
  e.clock.tick();
  assert.deepEqual(e.socket.events(), [
    {
      type: "heartbeat",
      generation: 1,
      revision: 1,
      playback_ms: 123,
      playback_audible: true,
    },
    {
      type: "heartbeat",
      generation: 1,
      revision: 2,
      playback_ms: 123,
      playback_audible: true,
    },
  ]);
  await e.client.end();
});

test("invalid playback telemetry fails closed instead of inventing a spoken confirmation watermark", async () => {
  const e = setup();
  await e.open("xai_realtime");
  e.media.report.playback_ms = Number.NaN;
  e.clock.tick();
  assert.deepEqual(e.socket.sent, []);
  assert.equal(e.client.isClosed, true);
});

test("Grok PCM metadata is paired with one bounded frame and supports generation-aware flush", async () => {
  const e = setup();
  await e.open("xai_realtime");
  const bytes = new ArrayBuffer(960);
  e.socket.receive({ type: "audio", generation: 1, end_ms: 20 });
  e.socket.receive(bytes);
  assert.deepEqual(e.media.enqueued, [{ bytes, endMs: 20 }]);
  e.socket.receive({ type: "audio", generation: 0, end_ms: 40 });
  e.socket.receive(new ArrayBuffer(960));
  assert.equal(
    e.media.enqueued.length,
    1,
    "stale generation pair is discarded",
  );
  e.socket.receive({ type: "audio_flush", generation: 0 });
  assert.equal(e.media.flushed, 0);
  e.socket.receive({ type: "audio_flush", generation: 1 });
  assert.equal(e.media.flushed, 1);
  await e.client.end();
});

for (const [name, events] of [
  ["unframed PCM", [new ArrayBuffer(960)]],
  [
    "odd PCM",
    [{ type: "audio", generation: 1, end_ms: 20 }, new ArrayBuffer(3)],
  ],
  [
    "oversized PCM",
    [{ type: "audio", generation: 1, end_ms: 20 }, new ArrayBuffer(1922)],
  ],
  [
    "duplicate metadata",
    [
      { type: "audio", generation: 1, end_ms: 20 },
      { type: "audio", generation: 1, end_ms: 40 },
    ],
  ],
  [
    "unbounded watermark",
    [{ type: "audio", generation: 1, end_ms: 1_830_001 }],
  ],
] as const) {
  test(`${name} ends Grok audio without accepting unverifiable output`, async () => {
    const e = setup();
    await e.open("xai_realtime");
    events.forEach((event) => e.socket.receive(event));
    assert.equal(e.client.isClosed, true);
    assert.equal(e.media.enqueued.length, 0);
    assert.deepEqual(
      e.controls.map((value) => value.input.action),
      ["end"],
    );
  });
}

test("Grok microphone frames are bounded and connection backpressure ends capture", async () => {
  const e = setup();
  await e.open("xai_realtime");
  e.client.hold(true);
  e.socket.bufferedAmount = 96_001;
  e.media.emit({ type: "pcm", bytes: new ArrayBuffer(960) });
  assert.equal(e.media.input, false);
  assert.equal(e.socket.sent.length, 1, "only ptt_begin was sent");
  assert.match(e.errors[0]!, /too slow/);
  const invalid = setup();
  await invalid.open("xai_realtime");
  invalid.client.hold(true);
  invalid.media.emit({ type: "pcm", bytes: new ArrayBuffer(3) });
  assert.equal(invalid.client.isClosed, true);
});

test("snapshot ID, generation and revision fences cannot restore stale input authority", async () => {
  const e = setup();
  await e.open("openai_live", "automatic");
  e.snapshot({ muted: true, control_revision: 2 });
  e.snapshot({ control_revision: 1 });
  e.snapshot({ id: requestId, generation: 10 });
  e.snapshot({ generation: 0, control_revision: 20 });
  assert.equal(e.media.input, false);
  assert.equal(e.snapshots.length, 2);
  e.snapshot({ generation: 2, control_revision: 0 });
  assert.equal(e.media.input, true);
  assert.equal(e.media.flushed, 1);
  await e.client.end();
});

test("mute suspends input immediately and late control responses cannot override newer snapshots", async () => {
  const e = setup();
  await e.open("openai_live", "automatic");
  const waiting = deferred<unknown>();
  e.responses.control = () => waiting.promise;
  const muting = e.client.mute(true);
  assert.equal(e.media.input, false);
  assert.equal(e.controls[0]!.input.expected_revision, 0);
  e.snapshot({ control_revision: 3, muted: true });
  waiting.resolve(session({ control_revision: 1, muted: false }));
  await muting;
  assert.equal(e.media.input, false);
  await e.client.end();
});

test("failed or mismatched mute controls end the call rather than reopening input", async () => {
  const e = setup();
  await e.open("openai_live", "automatic");
  e.responses.control = async (input) => {
    if (input.action === "end") return session();
    return session({ id: requestId });
  };
  await e.client.mute(true);
  assert.equal(e.media.input, false);
  assert.equal(e.client.isClosed, true);
  assert.deepEqual(
    e.controls.map((value) => value.input.action),
    ["mute", "end"],
  );
});

test("ending audio never cancels accepted work; Stop only cancels the chosen task", async () => {
  const e = setup();
  await e.open();
  await e.client.stopTask(requestId);
  assert.deepEqual(e.cancels, [{ thread: conversationId, id: requestId }]);
  await e.client.end();
  await e.client.end();
  await e.client.stopTask(sessionId);
  assert.equal(e.cancels.length, 1);
  assert.deepEqual(
    e.controls.map((value) => value.input.action),
    ["end"],
  );
  assert.equal(e.count().closed, 1);
});

test("closed server snapshot releases media once without another End or task cancellation", async () => {
  const e = setup();
  await e.open();
  e.snapshot({
    state: "closed",
    closed_at: "2026-10-16T00:01:00Z",
    end_reason: "server_update",
  });
  await e.client.end();
  assert.equal(e.client.isClosed, true);
  assert.equal(e.count().closed, 1);
  assert.deepEqual(e.controls, []);
  assert.deepEqual(e.cancels, []);
  assert.match(e.errors[0]!, /server update/);
});

test("speaker mute and audio resume remain local and never send server mute or task commands", async () => {
  const e = setup();
  await e.open();
  e.client.muteSpeaker(true);
  await e.client.resumeAudio();
  assert.deepEqual(e.media.speaker, [true, true]);
  assert.equal(e.media.resumed, 1);
  assert.deepEqual(e.controls, []);
  assert.deepEqual(e.cancels, []);
  await e.client.end();
});

test("malformed state and media errors are sanitized and fail closed", async () => {
  const e = setup();
  await e.open();
  e.socket.receive("{private invalid event");
  assert.equal(e.client.isClosed, true);
  assert.equal(e.errors[0]!.includes("private"), false);
  const mediaError = setup();
  await mediaError.open();
  mediaError.media.emit({ type: "error", message: "secret provider text" });
  assert.equal(mediaError.client.isClosed, true);
  assert.equal(mediaError.errors[0]!.includes("secret"), false);
});

for (const protocol of ["openai_live", "xai_realtime"] as const) {
  for (const [reason, blocked] of [
    ["muted", { muted: true }],
    ["input muted", { input_muted: true }],
    ["end requested", { end_requested: true }],
    ["closing", { state: "closing" }],
    ["starting", { state: "starting" }],
  ] as const) {
    test(`${protocol} requires a fresh press after an authoritative ${reason} snapshot`, async () => {
      const e = setup();
      await e.open(protocol);
      assert.equal(e.client.hold(true), true);
      assert.equal(e.media.input, true);
      e.snapshot({ ...blocked, control_revision: 1 });
      assert.equal(e.media.input, false);
      assert.equal(
        e.media.holds.at(-1),
        false,
        "input revocation restores output ducking too",
      );
      assert.equal(
        e.client.hold(true),
        false,
        "a blocked press cannot latch for later",
      );
      e.snapshot({ control_revision: 2 });
      assert.equal(
        e.media.input,
        false,
        "permission restoration cannot resume an old held press",
      );
      assert.equal(e.client.hold(false), false);
      if (protocol === "xai_realtime")
        assert.deepEqual(
          e.socket.events(),
          [{ type: "ptt_begin" }],
          "revocation and stale release must not commit interrupted speech",
        );
      assert.equal(e.client.hold(true), true);
      assert.equal(e.media.input, true);
      e.client.hold(false);
      await e.client.end();
    });
  }
  test(`${protocol} generation recovery cannot preserve a previous held press`, async () => {
    const e = setup();
    await e.open(protocol);
    e.client.hold(true);
    e.snapshot({ generation: 2 });
    assert.equal(e.media.input, false);
    assert.equal(e.media.holds.at(-1), false);
    e.snapshot({ generation: 2, control_revision: 1 });
    assert.equal(e.media.input, false);
    assert.equal(e.client.hold(true), true);
    assert.equal(e.media.input, true);
    await e.client.end();
  });
}

test("diagnostic failures release a held microphone and output ducking before closing", async () => {
  const e = setup();
  await e.open("xai_realtime");
  e.client.hold(true);
  e.media.emit({ type: "error", message: "private diagnostic text" });
  assert.equal(e.client.isClosed, true);
  assert.equal(e.media.input, false);
  assert.equal(e.media.holds.at(-1), false);
  assert.equal(e.client.hold(true), false);
  assert.equal(
    e.errors.some((message) => message.includes("private")),
    false,
  );
});
