import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistantHookTestHarness";
import {
  AssistantVoiceClient,
  type VoiceClientClock,
  type VoiceControlSocket,
  type VoiceMediaAdapter,
  type VoiceMediaEvent,
} from "./AssistantVoiceClient";
import type { VoicePreferences, VoiceSession } from "./voiceContracts";
import * as documentModule from "./voiceMediaDocument";

type Media = VoiceMediaAdapter & {
  readonly usesNativeMedia: boolean;
  connect(send: (message: Record<string, unknown>) => void): () => void;
  receive(raw: string): void;
};

function fixture() {
  const listeners = new Set<(event: VoiceMediaEvent) => void>();
  const state = {
    closed: 0,
    input: false,
    held: false,
    speakerMuted: false,
    resumed: 0,
    enqueued: 0,
  };
  const native: VoiceMediaAdapter = {
    prepare: async () => "offer",
    answer: async () => {},
    setInput: (enabled) => {
      state.input = enabled;
    },
    hold: (held) => {
      state.held = held;
    },
    muteSpeaker: (muted) => {
      state.speakerMuted = muted;
    },
    resume: async () => {
      state.resumed++;
    },
    enqueue: () => {
      state.enqueued++;
    },
    flush: () => {},
    playback: () => ({ playback_ms: 0, playback_audible: !state.speakerMuted }),
    close: () => {
      state.closed++;
      state.input = false;
      state.held = false;
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  const harness = hookHarness<unknown>({
    "react-native": { StyleSheet: { create: (value: unknown) => value } },
    "react-native-webview": { WebView: "WebView" },
    "./voiceMediaDocument": documentModule,
    "./createNativeOpenAIMedia": { createNativeOpenAIMedia: () => native },
  });
  const module = harness.load(
    fileURLToPath(new URL("./VoiceMediaBridge.tsx", import.meta.url)),
  ) as {
    NativeVoiceMedia: new (factory?: (() => VoiceMediaAdapter) | null) => Media;
    VoiceMediaBridge(props: { media: Media; origin: string }): unknown;
  };
  const media = new module.NativeVoiceMedia();
  return { state, native, listeners, harness, module, media };
}

const session: VoiceSession = {
  id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  created_at: "2026-11-09T00:00:00Z",
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
};
const preferences: VoicePreferences = {
  service_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  voice: "marin",
  input_mode: "automatic",
  language: null,
  notify_on_completion: false,
};

for (const settlement of ["terminal", "deadline"] as const) {
  test(`actual Bridge unmount preserves a quiet native End drain until ${settlement}, then releases it`, async (context) => {
    const f = fixture();
    context.after(() => {
      f.harness.unmount();
      f.media.close();
    });
    f.harness.mount(() =>
      f.module.VoiceMediaBridge({
        media: f.media,
        origin: "https://nyx.example",
      }),
    );
    const timers = new Map<number, () => void>();
    let sequence = 0;
    const clock: VoiceClientClock = {
      setInterval: () => 1,
      clearInterval: () => {},
      setTimeout: (callback) => {
        const id = ++sequence;
        timers.set(id, callback);
        return id;
      },
      clearTimeout: (id) => {
        timers.delete(id as number);
      },
    };
    const socket: VoiceControlSocket = {
      readyState: 1,
      bufferedAmount: 0,
      binaryType: "",
      onmessage: null,
      onclose: null,
      onerror: null,
      send: () => {},
      close: () => {
        socket.readyState = 3;
      },
    };
    let controls = 0;
    let endSignal: AbortSignal | undefined;
    const cleanup: (VoiceSession | null)[] = [];
    const client = new AssistantVoiceClient({
      conversationId: `nyxa-${"a".repeat(32)}`,
      media: f.media,
      clock,
      api: {
        start: async () => ({ session, sdp_answer: "answer" }),
        active: async () => ({ session: null }),
        control: async (_thread, _session, _input, signal) => {
          controls++;
          endSignal = signal;
          return {
            ...session,
            state: "closing",
            end_requested: true,
            control_revision: 1,
          };
        },
        cancelRequest: async () => {},
      },
      createSocket: async () => socket,
      onSnapshot: () => {},
      onError: () =>
        assert.fail("quiet draining must not publish a live error"),
      onCleanup: (value) => cleanup.push(value),
      randomId: () => "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
    });
    await client.start(preferences);
    for (const listener of f.listeners) listener({ type: "provider-ready" });
    socket.onmessage?.({
      data: JSON.stringify({
        type: "snapshot",
        session,
        captions: [],
        tasks: [],
      }),
    });
    assert.equal(f.state.input, true);

    const ending = client.end();
    assert.equal(f.state.input, false);
    assert.equal(f.state.speakerMuted, true);
    f.harness.unmount(); // Executes the actual media.connect() cleanup.
    assert.equal(f.state.closed, 0);
    assert.equal(f.media.usesNativeMedia, true);
    assert.equal(socket.readyState, 1);
    f.media.setInput(true);
    f.media.hold(true);
    f.media.muteSpeaker(false);
    await f.media.resume();
    f.media.enqueue(new ArrayBuffer(960), 1);
    assert.equal(f.state.input, false);
    assert.equal(f.state.held, false);
    assert.equal(f.state.speakerMuted, true);
    assert.equal(f.state.resumed, 0);
    assert.equal(f.state.enqueued, 0);
    for (let step = 0; step < 8; step++) await Promise.resolve();
    if (settlement === "terminal") {
      socket.onmessage?.({
        data: JSON.stringify({
          type: "snapshot",
          session: {
            ...session,
            state: "closed",
            end_requested: true,
            control_revision: 1,
          },
          captions: [],
          tasks: [],
        }),
      });
    } else {
      for (const callback of timers.values()) callback();
    }
    await ending;
    assert.equal(f.state.closed, 1);
    assert.equal(f.media.usesNativeMedia, false);
    assert.equal(socket.readyState, 3);
    assert.equal(endSignal?.aborted, true);
    assert.equal(controls, 1);
    assert.equal(timers.size, 0);
    assert.equal(
      cleanup.at(-1)?.state ?? null,
      settlement === "terminal" ? null : "closing",
    );

    // A subsequent native generation does not inherit the old drain exemption.
    const disconnect = f.media.connect(() => {});
    await f.media.prepare("openai_live", new AbortController().signal);
    disconnect();
    assert.equal(f.state.closed, 2);
    assert.equal(f.media.usesNativeMedia, false);
  });
}

test("ordinary native Bridge unmount still releases capture immediately", async () => {
  const f = fixture();
  f.harness.mount(() =>
    f.module.VoiceMediaBridge({
      media: f.media,
      origin: "https://nyx.example",
    }),
  );
  await f.media.prepare("openai_live", new AbortController().signal);
  f.media.setInput(true);
  f.harness.unmount();
  assert.equal(f.state.closed, 1);
  assert.equal(f.state.input, false);
  assert.equal(f.media.usesNativeMedia, false);
});

test("beginEndDrain never exempts a WebView engine from disconnect cleanup", async () => {
  const f = fixture();
  const media = new f.module.NativeVoiceMedia(null);
  const messages: Record<string, unknown>[] = [];
  const disconnect = media.connect((message) => {
    messages.push(message);
    if (message.type === "prepare")
      media.receive(
        JSON.stringify({
          type: "response",
          generation: message.generation,
          requestId: message.requestId,
          result: "",
        }),
      );
  });
  media.receive(JSON.stringify({ type: "ready" }));
  await media.prepare("xai_realtime", new AbortController().signal);
  media.beginEndDrain?.();
  disconnect();
  assert.equal(messages.at(-1)?.type, "close");
  assert.equal(f.state.closed, 0);
  assert.equal(media.usesNativeMedia, false);
});
