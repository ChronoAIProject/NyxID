import assert from "node:assert/strict";
import test from "node:test";
import { AssistantDictationClient } from "./AssistantDictationClient";
import type {
  VoiceControlSocket,
  VoiceMediaEvent,
  VoiceMediaAdapter,
} from "./AssistantVoiceClient";
const thread = "nyxa-" + "a".repeat(32);
const id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const preferences = {
  service_id: id,
  connection_id: null,
  key_source: "platform" as const,
  model: "gpt-live-transcribe",
  voice: null,
  language: null,
  input_mode: "push_to_talk" as const,
  notify_on_completion: false,
};
function setup() {
  const events: string[] = [],
    sent: (string | ArrayBuffer)[] = [],
    errors: string[] = [],
    texts: any[] = [];
  let listener: (event: VoiceMediaEvent) => void = () => {};
  const media = {
    prepare: async (protocol: string) => {
      assert.equal(protocol, "dictation");
      events.push("prepare");
      return "";
    },
    close: () => events.push("close"),
    setInput: (v: boolean) => events.push("input:" + v),
    subscribe: (cb: typeof listener) => {
      listener = cb;
      return () => events.push("unsubscribe");
    },
    answer: async () => {},
    hold: () => {},
    muteSpeaker: () => {},
    resume: async () => {},
    enqueue: () => {},
    flush: () => {},
    playback: () => ({ playback_ms: 0, playback_audible: false }),
  } satisfies VoiceMediaAdapter;
  const socket: VoiceControlSocket = {
    readyState: 0,
    bufferedAmount: 0,
    binaryType: "blob",
    onopen: null,
    onmessage: null,
    onclose: null,
    onerror: null,
    send: (data) => sent.push(data),
    close: () => events.push("socket-close"),
  };
  const client = new AssistantDictationClient({
    media,
    randomId: () => id,
    createSocket: async (path) => {
      assert.equal(
        path,
        `/assistant/nyxagent/conversations/${thread}/dictation`,
      );
      events.push("socket");
      return socket;
    },
    onText: (t) => texts.push(t),
    onPhase: (p) => events.push(p),
    onError: (e) => errors.push(e),
  });
  const emit = (value: unknown) =>
    socket.onmessage?.({ data: JSON.stringify(value) });
  const ready = () =>
    emit({ type: "ready", session_id: id, model: "gpt-live-transcribe" });
  const open = () => {
    socket.readyState = 1;
    socket.onopen?.();
  };
  return {
    client,
    socket,
    sent,
    events,
    errors,
    texts,
    emit,
    ready,
    open,
    pcm: () => listener({ type: "pcm", bytes: new ArrayBuffer(960) }),
  };
}
test("dictation matches web initial request, PCM24k and stop protocol, with ordered completed review", async () => {
  const x = setup();
  await x.client.start(thread, preferences);
  assert.equal(x.sent.length, 0);
  x.open();
  assert.deepEqual(JSON.parse(x.sent[0] as string), {
    client_request_id: id,
    preferences,
  });
  x.pcm();
  assert.equal(x.sent.length, 1);
  x.ready();
  x.pcm();
  assert.equal((x.sent[1] as ArrayBuffer).byteLength, 960);
  x.emit({ type: "delta", item_id: "b", order: 2, text: "there" });
  x.emit({ type: "completed", item_id: "a", order: 1, text: "Hello" });
  x.emit({ type: "completed", item_id: "b", order: 2, text: "world" });
  x.emit({ type: "delta", item_id: "b", order: 2, text: " ignored" });
  assert.deepEqual(x.texts.at(-1), { final: "Hello world", interim: "" });
  x.client.stop();
  assert.equal(x.sent.at(-1), '{"type":"stop"}');
  assert.ok(x.events.indexOf("close") < x.events.indexOf("stopping"));
  x.emit({ type: "stopped", confirmed: true });
  assert.equal(x.events.at(-1), "review");
  assert.deepEqual(x.errors, []);
});
test("cancel closes media immediately, drains server boundedly and ignores late transcript", async () => {
  const x = setup();
  await x.client.start(thread, preferences);
  x.open();
  x.ready();
  x.client.cancel();
  const count = x.texts.length;
  x.emit({ type: "completed", item_id: "late", order: 0, text: "private" });
  x.emit({ type: "stopped", confirmed: true });
  assert.equal(x.texts.length, count);
  assert.equal(x.events.includes("review"), false);
  assert.ok(x.events.includes("socket-close"));
});
test("early stop waits for ready then sends exactly one stop, without forwarding audio", async () => {
  const x = setup();
  await x.client.start(thread, preferences);
  x.open();
  x.client.stop();
  x.ready();
  x.pcm();
  assert.equal(x.sent.filter((v) => v === '{"type":"stop"}').length, 1);
  assert.equal(
    x.sent.some((v) => v instanceof ArrayBuffer),
    false,
  );
  x.emit({ type: "stopped", confirmed: true });
});
test("unverified events, oversized transcript and backpressure fail closed", async () => {
  for (const kind of ["invalid", "overflow", "backpressure"]) {
    const x = setup();
    await x.client.start(thread, preferences);
    x.open();
    x.ready();
    if (kind === "invalid")
      x.emit({ type: "completed", item_id: "x", order: -1, text: "bad" });
    if (kind === "overflow") {
      x.emit({
        type: "delta",
        item_id: "x",
        order: 0,
        text: "x".repeat(32000),
      });
      x.emit({ type: "delta", item_id: "x", order: 0, text: "more" });
    }
    if (kind === "backpressure") {
      x.socket.bufferedAmount = 48001;
      x.pcm();
    }
    assert.equal(x.errors.length, 1, kind);
    assert.ok(x.events.includes("socket-close"));
  }
});
test("socket loss never reconnects or falls back to OS recognition", async () => {
  const x = setup();
  await x.client.start(thread, preferences);
  x.open();
  x.ready();
  x.socket.onclose?.();
  assert.equal(x.errors.length, 1);
  assert.equal(x.events.filter((e) => e === "socket").length, 1);
});
