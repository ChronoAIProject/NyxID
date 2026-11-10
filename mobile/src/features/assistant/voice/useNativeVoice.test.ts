import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistantHookTestHarness";
import type { AssistantVoiceClientOptions } from "./AssistantVoiceClient";
import type {
  VoicePreferences,
  VoiceSession,
  VoiceSnapshot,
} from "./voiceContracts";
import type { VoiceStartRecovery } from "./voiceStartError";

type VoiceHook = ReturnType<typeof import("./useNativeVoice").useNativeVoice>;
const firstThread = "nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const secondThread = "nyxa-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const firstId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const secondId = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const preferences: VoicePreferences = {
  service_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  voice: "alloy",
  input_mode: "push_to_talk",
  language: null,
  notify_on_completion: false,
};
const creditRecovery: VoiceStartRecovery = {
  kind: "credits",
  message: "Review available credits in Billing before trying voice again.",
};
function session(changes: Partial<VoiceSession> = {}): VoiceSession {
  return {
    id: firstId,
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
function snapshot(changes: Partial<VoiceSession> = {}): VoiceSnapshot {
  return {
    type: "snapshot",
    session: session(changes),
    captions: [],
    tasks: [],
  };
}

/** The real hook executes; only native permission, media and network edges are replaced. */
function setup(context: TestContext) {
  let epoch = 1;
  let thread = firstThread;
  let optionCalls = 0;
  let savedCalls = 0;
  let permissionCalls = 0;
  let refreshCalls = 0;
  const signals: AbortSignal[] = [];
  const controls: { thread: string; id: string; action: string }[] = [];
  const controlSignals: (AbortSignal | undefined)[] = [];
  const cancellations: { thread: string; id: string; signal: AbortSignal }[] =
    [];
  const activeReads: string[] = [];
  const statusReads: { thread: string; id: string; signal?: AbortSignal }[] =
    [];
  const appListeners = new Set<(state: string) => void>();
  const native = {
    currentState: "active",
    addEventListener: (_event: string, callback: (state: string) => void) => {
      appListeners.add(callback);
      return { remove: () => appListeners.delete(callback) };
    },
  };
  const available = {
    ...preferences,
    available: true,
    voice: {
      protocol: "openai_live",
      voices: [{ id: "alloy", label: "Alloy" }],
    },
  };
  const responses = {
    options: async (): Promise<any> => ({ options: [available] }),
    save: async (): Promise<unknown> => ({}),
    permission: async () => ({ granted: true }),
    start: async (_client: Client): Promise<void> => {},
    end: async (client: Client): Promise<void> => {
      client.options.media.close();
      client.options.onClosed?.();
    },
    control: async (): Promise<unknown> => session({ state: "closed" }),
    active: async (): Promise<any> => ({ session: null }),
    status: async (): Promise<unknown> => session(),
    cancel: async (): Promise<void> => {},
    resume: async (_client: Client): Promise<void> => {},
    hold: (pressed: boolean) => pressed,
  };
  const api = {
    options: async (_thread: string, signal: AbortSignal) => {
      optionCalls++;
      signals.push(signal);
      return responses.options();
    },
    savePreferences: async () => {
      savedCalls++;
      return responses.save();
    },
    control: async (
      selectedThread: string,
      id: string,
      input: { action: string },
      signal?: AbortSignal,
    ) => {
      controls.push({
        thread: selectedThread,
        id,
        action: input.action,
      });
      controlSignals.push(signal);
      return responses.control();
    },
    active: async (selectedThread: string) => {
      activeReads.push(selectedThread);
      return responses.active();
    },
    status: async (
      selectedThread: string,
      id: string,
      signal?: AbortSignal,
    ) => {
      statusReads.push({ thread: selectedThread, id, signal });
      return responses.status();
    },
    cancelRequest: async (
      selectedThread: string,
      id: string,
      signal: AbortSignal,
    ) => {
      cancellations.push({ thread: selectedThread, id, signal });
      return responses.cancel();
    },
  };
  const clients: Client[] = [];
  const media: Media[] = [];
  class Media {
    id = String(media.length + 1);
    closed = 0;
    constructor() {
      media.push(this);
    }
    close() {
      this.closed++;
    }
  }
  class Client {
    pendingEnd?: VoiceSession;
    pendingStart = false;
    isClosed = false;
    starts = 0;
    ends = 0;
    holds: boolean[] = [];
    speaker: boolean[] = [];
    mutes: boolean[] = [];
    stops: string[] = [];
    resumed = 0;
    constructor(readonly options: AssistantVoiceClientOptions) {
      clients.push(this);
    }
    async start() {
      this.starts++;
      return responses.start(this);
    }
    async end() {
      this.ends++;
      this.isClosed = true;
      return responses.end(this);
    }
    hold(value: boolean) {
      this.holds.push(value);
      return responses.hold(value);
    }
    muteSpeaker(value: boolean) {
      this.speaker.push(value);
    }
    async mute(value: boolean) {
      this.mutes.push(value);
    }
    async resumeAudio() {
      this.resumed++;
      return responses.resume(this);
    }
    async stopTask(id: string) {
      this.stops.push(id);
    }
    closed() {
      this.isClosed = true;
      this.options.onClosed?.();
    }
  }
  let environment = { epoch, api, createSocket: async () => undefined };
  const harness = hookHarness<VoiceHook>({
    "react-native": { AppState: native },
    "expo-camera": {
      Camera: {
        requestMicrophonePermissionsAsync: async () => {
          permissionCalls++;
          return responses.permission();
        },
      },
    },
    "expo-crypto": { randomUUID: () => secondId },
    "../../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "./AssistantVoiceClient": { AssistantVoiceClient: Client },
    "./VoiceMediaBridge": { NativeVoiceMedia: Media },
  });
  const loaded = harness.load(
    fileURLToPath(new URL("./useNativeVoice.ts", import.meta.url)),
  ) as {
    useNativeVoice: (
      thread: string,
      environment: any,
      onUpdate: () => void,
    ) => VoiceHook;
  };
  const render = () =>
    loaded.useNativeVoice(thread, environment, () => {
      refreshCalls++;
    });
  harness.mount(render);
  context.after(() => harness.unmount());
  const start = async () => {
    await harness.value().start(preferences, "openai_live");
    return harness.flush();
  };
  const connect = async () => {
    await start();
    clients.at(-1)!.options.onSnapshot(snapshot());
    clients.at(-1)!.options.onReady?.();
    return harness.flush();
  };
  return {
    api,
    harness,
    clients,
    media,
    controls,
    controlSignals,
    cancellations,
    activeReads,
    statusReads,
    available,
    responses,
    native,
    signals,
    start,
    connect,
    remount: () => harness.mount(render),
    counts: () => ({
      optionCalls,
      savedCalls,
      permissionCalls,
      refreshCalls,
      listeners: appListeners.size,
    }),
    appState: (state: string) => {
      native.currentState = state;
      appListeners.forEach((listener) => listener(state));
      return harness.flush();
    },
    setEpoch: (value: number) => {
      epoch = value;
    },
    replaceScope: (nextThread: string = thread, nextEpoch: number = epoch) => {
      thread = nextThread;
      epoch = nextEpoch;
      environment = { ...environment, epoch: nextEpoch };
      return harness.render();
    },
  };
}

test("duplicate Start presses share the same pending preflight and create one client", async (context) => {
  const e = setup(context);
  const waiting = deferred<any>();
  e.responses.options = () => waiting.promise;
  const start = e.harness.value().start;
  const first = start(preferences, "openai_live");
  const second = start(preferences, "openai_live");
  assert.equal(e.counts().optionCalls, 1);
  waiting.resolve({ options: [e.available] });
  await Promise.all([first, second]);
  assert.equal(e.counts().savedCalls, 1);
  assert.equal(e.counts().permissionCalls, 1);
  assert.equal(e.clients.length, 1);
  assert.equal(e.clients[0]!.starts, 1);
});

test("active control metadata stays Connecting until the client confirms media and control readiness", async (context) => {
  const e = setup(context);
  const initial = await e.start();
  assert.equal(initial.starting, true);
  assert.equal(initial.connected, false);
  const client = e.clients[0]!;
  client.options.onSnapshot(snapshot());
  const waiting = e.harness.flush();
  assert.equal(waiting.snapshot?.session.state, "active");
  assert.equal(waiting.starting, true);
  assert.equal(waiting.connected, false);
  client.options.onReady?.();
  const connected = e.harness.flush();
  assert.equal(connected.starting, false);
  assert.equal(connected.connected, true);
  assert.equal(connected.holding, false);
  client.options.onSnapshot(snapshot({ muted: true, input_muted: true }));
  assert.equal(e.harness.flush().connected, true);
});

test("capture state follows client input notifications independently of readiness and captions", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  assert.equal(e.harness.value().connected, true);
  assert.equal(e.harness.value().inputEnabled, false);
  client.options.onInputChanged?.(true);
  assert.equal(e.harness.flush().inputEnabled, true);
  client.options.onInputChanged?.(false);
  client.options.onSnapshot({
    ...snapshot(),
    captions: [
      {
        id: "prior",
        speaker: "user",
        text: "An older utterance",
        sealed: true,
        complete: true,
      },
    ],
  });
  assert.equal(e.harness.flush().inputEnabled, false);
  assert.equal(e.harness.value().connected, true);
});

for (const exit of [
  "end",
  "background",
  "thread",
  "account",
  "unmount",
] as const) {
  test(`${exit} clears capture immediately and rejects a late input notification`, async (context) => {
    const e = setup(context);
    await e.connect();
    const client = e.clients[0]!;
    client.options.onInputChanged?.(true);
    assert.equal(e.harness.flush().inputEnabled, true);
    if (exit === "end") {
      client.pendingEnd = session({ state: "closing", end_requested: true });
      const ending = e.harness.value().end();
      assert.equal(e.harness.flush().inputEnabled, false);
      assert.equal(await ending, false);
    } else if (exit === "background") e.appState("background");
    else if (exit === "thread") e.replaceScope(secondThread);
    else if (exit === "account") e.replaceScope(firstThread, 2);
    else {
      e.harness.unmount();
      e.remount();
    }
    assert.equal(e.harness.flush().inputEnabled, false);
    client.options.onInputChanged?.(true);
    assert.equal(e.harness.flush().inputEnabled, false);
  });
}

test("a readiness timeout ends Connecting and late readiness cannot restore the closed client", async (context) => {
  const e = setup(context);
  await e.start();
  const client = e.clients[0]!;
  client.options.onSnapshot(snapshot());
  client.options.onError(
    "Voice connection timed out. Start a new call to reconnect.",
  );
  client.closed();
  let state = e.harness.flush();
  assert.equal(state.starting, false);
  assert.equal(state.connected, false);
  assert.match(state.error!, /timed out/);
  client.options.onReady?.();
  state = e.harness.flush();
  assert.equal(state.starting, false);
  assert.equal(state.connected, false);
});

for (const change of ["end", "thread", "account", "background"] as const) {
  test(`late readiness after ${change} cannot connect an obsolete call attempt`, async (context) => {
    const e = setup(context);
    await e.start();
    const old = e.clients[0]!;
    if (change === "end") await e.harness.value().end();
    else if (change === "thread") e.replaceScope(secondThread);
    else if (change === "account") e.replaceScope(firstThread, 2);
    else e.appState("background");
    old.options.onReady?.();
    let state = e.harness.flush();
    assert.equal(state.starting, false);
    assert.equal(state.connected, false);
    if (change === "background") e.appState("active");
    await e.start();
    old.options.onReady?.();
    state = e.harness.flush();
    assert.equal(state.starting, true);
    assert.equal(state.connected, false);
    e.clients[1]!.options.onReady?.();
    assert.equal(e.harness.flush().connected, true);
  });
}

test("Resume clears the playback warning locally and a blocked retry keeps the same call usable", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.options.onError("Tap Resume audio to hear the assistant.");
  let state = e.harness.flush();
  assert.equal(state.connected, true);
  assert.equal(state.starting, false);
  await state.resumeAudio();
  state = e.harness.flush();
  assert.equal(state.error, undefined);
  assert.equal(state.connected, true);
  client.options.onError("Tap Resume audio to hear the assistant.");
  state = e.harness.flush();
  e.responses.resume = async (current) => {
    current.options.onError("Tap Resume audio to hear the assistant.");
  };
  await state.resumeAudio();
  state = e.harness.flush();
  assert.equal(state.error, "Tap Resume audio to hear the assistant.");
  assert.equal(state.connected, true);
  assert.equal(client.resumed, 2);
  assert.equal(client.ends, 0);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

for (const change of ["thread", "account", "unmount"] as const) {
  test(`a retained Start callback after ${change} cannot begin another call`, async (context) => {
    const e = setup(context);
    const retainedStart = e.harness.value().start;
    if (change === "thread") e.replaceScope(secondThread);
    else if (change === "account") e.replaceScope(firstThread, 2);
    else e.harness.unmount();
    await retainedStart(preferences, "openai_live");
    assert.equal(e.counts().optionCalls, 0);
    assert.equal(e.counts().savedCalls, 0);
    assert.equal(e.counts().permissionCalls, 0);
    assert.equal(e.clients.length, 0);
  });
}

test("starting while backgrounded or after account invalidation performs no preflight", async (context) => {
  const e = setup(context);
  e.appState("background");
  await e.start();
  assert.equal(e.counts().optionCalls, 0);
  e.appState("active");
  e.setEpoch(2);
  await e.start();
  assert.equal(e.counts().optionCalls, 0);
  assert.equal(e.counts().savedCalls, 0);
  assert.equal(e.counts().permissionCalls, 0);
});

test("retained controls from another chat cannot affect the current call", async (context) => {
  const e = setup(context);
  await e.connect();
  const retained = e.harness.value();
  e.replaceScope(secondThread);
  await e.connect();
  const current = e.clients[1]!;
  retained.hold(true);
  await retained.mute(true);
  retained.muteSpeaker(true);
  await retained.resumeAudio();
  await retained.stopTask(firstId);
  await retained.end();
  assert.deepEqual(current.holds, []);
  assert.deepEqual(current.mutes, []);
  assert.deepEqual(current.speaker, []);
  assert.deepEqual(current.stops, []);
  assert.equal(current.resumed, 0);
  assert.equal(current.ends, 0);
  assert.equal(e.harness.flush().connected, true);
});

test("after an account change the same chat can still silence and end its open call", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  e.setEpoch(2);
  const controls = e.harness.value();
  controls.hold(true);
  await controls.mute(false);
  assert.deepEqual(client.holds, []);
  assert.deepEqual(client.mutes, []);
  await controls.mute(true);
  controls.muteSpeaker(true);
  assert.deepEqual(client.mutes, [true]);
  assert.deepEqual(client.speaker, [true]);
  await controls.end();
  assert.equal(client.ends, 1);
  assert.equal(e.harness.flush().connected, false);
  assert.equal(e.counts().refreshCalls, 0);
});

test("a retained conflict action cannot end a call after switching chats", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  const retainedEnd = e.harness.value().endConflict;
  e.replaceScope(secondThread);
  await retainedEnd();
  assert.deepEqual(e.controls, []);
  assert.deepEqual(e.activeReads, []);
});

test("saving preferences must succeed before any microphone permission or media acquisition", async (context) => {
  const e = setup(context);
  e.responses.save = async () => {
    throw new Error("Save failed");
  };
  const state = await e.start();
  assert.equal(e.counts().savedCalls, 1);
  assert.equal(e.counts().permissionCalls, 0);
  assert.equal(e.clients.length, 0);
  assert.equal(e.media.length, 0);
  assert.equal(state.starting, false);
  assert.ok(state.error);
});

test("a source revoked during preflight prevents preference save and microphone prompts", async (context) => {
  const e = setup(context);
  e.available.available = false;
  const state = await e.start();
  assert.equal(e.counts().savedCalls, 0);
  assert.equal(e.counts().permissionCalls, 0);
  assert.equal(e.clients.length, 0);
  assert.ok(state.error);
});

test("permission denial keeps text usable and never constructs a provider client", async (context) => {
  const e = setup(context);
  e.responses.permission = async () => ({ granted: false });
  const state = await e.start();
  assert.equal(e.counts().permissionCalls, 1);
  assert.equal(e.clients.length, 0);
  assert.equal(e.media.length, 0);
  assert.equal(state.starting, false);
  assert.match(state.error!, /Microphone access is off/);
});

test("End during option preflight aborts the request and rejects a late successful response", async (context) => {
  const e = setup(context);
  const waiting = deferred<any>();
  e.responses.options = () => waiting.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.value().end();
  assert.equal(e.signals[0]!.aborted, true);
  waiting.resolve({ options: [e.available] });
  await pending;
  assert.equal(e.counts().savedCalls, 0);
  assert.equal(e.counts().permissionCalls, 0);
  assert.equal(e.clients.length, 0);
  assert.equal(e.harness.flush().starting, false);
});

test("account replacement during save cannot prompt for mic or open a provider", async (context) => {
  const e = setup(context);
  const waiting = deferred<unknown>();
  e.responses.save = () => waiting.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  assert.equal(e.counts().savedCalls, 1);
  e.replaceScope(firstThread, 2);
  waiting.resolve({});
  await pending;
  assert.equal(e.counts().permissionCalls, 0);
  assert.equal(e.clients.length, 0);
});

test("system permission inactivity is tolerated, but actual background cancels pending permission", async (context) => {
  const e = setup(context);
  const waiting = deferred<{ granted: boolean }>();
  e.responses.permission = () => waiting.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  assert.equal(e.counts().permissionCalls, 1);
  assert.equal(e.appState("inactive").starting, true);
  assert.equal(e.signals[0]!.aborted, false);
  assert.equal(e.appState("background").starting, false);
  waiting.resolve({ granted: true });
  await pending;
  assert.equal(e.clients.length, 0);
});

test("returning from the native permission prompt can continue the same start attempt", async (context) => {
  const e = setup(context);
  const waiting = deferred<{ granted: boolean }>();
  e.responses.permission = () => waiting.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  e.appState("inactive");
  e.appState("active");
  waiting.resolve({ granted: true });
  await pending;
  assert.equal(e.clients.length, 1);
  assert.equal(e.clients[0]!.starts, 1);
});

test("iOS can resolve permission while inactive and wait for the later active event before acquiring media", async (context) => {
  const e = setup(context);
  const permission = deferred<{ granted: boolean }>();
  e.responses.permission = () => permission.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  e.appState("inactive");
  permission.resolve({ granted: true });
  await e.harness.settle();
  assert.equal(e.harness.value().starting, true);
  assert.equal(e.harness.value().error, undefined);
  assert.equal(e.media.length, 0);
  assert.equal(e.clients.length, 0);
  assert.equal(
    e.counts().listeners,
    2,
    "a temporary foreground observer owns the pending handoff",
  );
  e.appState("inactive");
  assert.equal(
    e.signals[0]!.aborted,
    false,
    "the post-permission inactive interval remains part of this attempt",
  );
  e.appState("active");
  await pending;
  assert.equal(e.clients.length, 1);
  assert.equal(e.clients[0]!.starts, 1);
  assert.equal(
    e.counts().listeners,
    1,
    "active handoff removes its temporary observer",
  );
});

test("foreground handoff has a five-second bound, cleans its listener and allows explicit retry", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  const permission = deferred<{ granted: boolean }>();
  e.responses.permission = () => permission.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  e.appState("inactive");
  permission.resolve({ granted: true });
  await e.harness.settle();
  context.mock.timers.tick(4999);
  await e.harness.settle();
  assert.equal(e.harness.value().starting, true);
  assert.equal(e.clients.length, 0);
  context.mock.timers.tick(1);
  await pending;
  const failed = e.harness.flush();
  assert.equal(failed.starting, false);
  assert.match(failed.error!, /did not return to the foreground/);
  assert.equal(e.counts().listeners, 1);
  e.appState("active");
  assert.equal(
    e.clients.length,
    0,
    "a late active event must not revive the expired attempt",
  );
  e.responses.permission = async () => ({ granted: true });
  await e.start();
  assert.equal(
    e.clients.length,
    1,
    "timeout releases the busy lock for a new explicit start",
  );
});

for (const reason of [
  "end",
  "background",
  "thread",
  "account",
  "unmount",
] as const) {
  test(`${reason} while waiting for foreground cancels the handoff and never creates media`, async (context) => {
    const e = setup(context);
    const permission = deferred<{ granted: boolean }>();
    e.responses.permission = () => permission.promise;
    const pending = e.harness.value().start(preferences, "openai_live");
    await e.harness.settle();
    e.appState("inactive");
    permission.resolve({ granted: true });
    await e.harness.settle();
    assert.equal(e.counts().listeners, 2);
    if (reason === "end") await e.harness.value().end();
    else if (reason === "background") e.appState("background");
    else if (reason === "thread") e.replaceScope(secondThread);
    else if (reason === "account") e.replaceScope(firstThread, 2);
    else e.harness.unmount();
    await pending;
    assert.equal(e.clients.length, 0);
    assert.equal(e.media.length, 0);
    assert.equal(e.counts().listeners, reason === "unmount" ? 0 : 1);
    e.appState("active");
    assert.equal(
      e.clients.length,
      0,
      "late foreground notification cannot revive a cancelled attempt",
    );
  });
}

test("a cancelled permission request cannot release a newer attempt's inactive exemption or busy lock", async (context) => {
  const e = setup(context);
  const firstPermission = deferred<{ granted: boolean }>();
  const secondPermission = deferred<{ granted: boolean }>();
  e.responses.permission = () => firstPermission.promise;
  const first = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  await e.harness.value().end();
  e.harness.flush();
  e.responses.permission = () => secondPermission.promise;
  const second = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  assert.equal(e.counts().permissionCalls, 2);
  firstPermission.resolve({ granted: true });
  await first;
  e.appState("inactive");
  assert.equal(e.signals[1]!.aborted, false);
  assert.equal(e.harness.value().starting, true);
  await e.harness.value().start(preferences, "openai_live");
  assert.equal(
    e.counts().optionCalls,
    2,
    "the stale finally must not release the second start's busy guard",
  );
  secondPermission.resolve({ granted: true });
  await e.harness.settle();
  assert.equal(e.clients.length, 0);
  e.appState("active");
  await second;
  assert.equal(e.clients.length, 1);
});

test("a confirmed quiet-call end replaces a media-first error with the terminal snapshot", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.options.onSnapshot(snapshot({ muted: true, idle_warning: true }));
  client.options.onError(
    "Voice media stopped. Check microphone permission and start a new call.",
  );
  client.pendingEnd = session();
  client.closed();
  const stopped = e.harness.flush();
  assert.equal(stopped.snapshot?.session.state, "active");
  assert.match(stopped.error!, /Voice media stopped/);

  client.pendingEnd = undefined;
  client.options.onSnapshot(
    snapshot({
      state: "closed",
      closed_at: "2026-10-16T00:04:25Z",
      end_reason: "idle",
    }),
  );
  client.options.onConflict?.(null);
  const ended = e.harness.flush();
  assert.equal(ended.snapshot?.session.state, "closed");
  assert.equal(ended.error, undefined);
  assert.equal(ended.conflict, null);
  assert.equal(ended.connected, false);
  assert.equal(ended.inputEnabled, false);
  assert.equal(e.clients.length, 1);
});

test("a confirmed ended call clears an obsolete playback hint without clearing a provider failure", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.options.onError(
    "Tap Resume audio to hear the assistant.",
    undefined,
    "playback",
  );
  client.options.onSnapshot(
    snapshot({ state: "closed", end_reason: "time_limit" }),
  );
  assert.equal(e.harness.flush().error, undefined);
  client.options.onError("Voice provider rejected the session", creditRecovery);
  client.options.onSnapshot(
    snapshot({ state: "closed", end_reason: "time_limit" }),
  );
  assert.equal(e.harness.flush().error, "Voice provider rejected the session");
  assert.deepEqual(e.harness.value().recovery, creditRecovery);
  assert.equal(e.clients.length, 1);
});

test("late quiet-call terminal callbacks cannot replace another account or chat", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  e.replaceScope(secondThread, 2);
  client.options.onSnapshot(snapshot({ state: "closed", end_reason: "idle" }));
  client.options.onConflict?.(null);
  const state = e.harness.flush();
  assert.equal(state.snapshot, null);
  assert.equal(state.error, undefined);
  assert.equal(state.connected, false);
  assert.equal(e.clients.length, 1);
});

test("a terminal cleanup does not clear an unrelated failed-start diagnostic", async (context) => {
  const e = setup(context);
  await e.start();
  const client = e.clients[0]!;
  client.options.onError("Voice provider rejected the session", creditRecovery);
  client.closed();
  client.options.onSnapshot(snapshot({ state: "closed", end_reason: "idle" }));
  const state = e.harness.flush();
  assert.equal(state.error, "Voice provider rejected the session");
  assert.deepEqual(state.recovery, creditRecovery);
});

test("backgrounding an active call ends media and cannot restart through late callbacks", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  e.harness.value().hold(true);
  assert.equal(e.harness.flush().holding, true);
  const state = e.appState("background");
  assert.equal(client.ends, 1);
  assert.equal(state.connected, false);
  assert.equal(state.holding, false);
  assert.equal(
    state.error,
    undefined,
    "normal app backgrounding is not a voice failure",
  );
  client.options.onSnapshot(snapshot({ generation: 2 }));
  assert.equal(e.harness.flush().connected, false);
  assert.equal(e.clients.length, 1);
});

test("reopening voice automatically verifies retained active metadata and releases a finished call", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session(), secondThread);
    client.closed();
  };
  await e.start();
  e.harness.unmount();
  assert.equal(e.remount().conflict?.id, firstId);
  e.responses.status = async () => session({ state: "closed" });
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.equal(e.statusReads.length, 1);
  assert.equal(e.statusReads[0]?.thread, secondThread);
  assert.equal(e.harness.value().conflict, null);
  assert.equal(e.harness.value().confirmedEnded, true);
  assert.equal(e.harness.value().error, undefined);
  assert.deepEqual(
    e.controls,
    [],
    "verification never ends another chat's call",
  );
  assert.equal(
    e.clients.length,
    1,
    "verification never creates a replacement call",
  );
  e.responses.start = async () => {};
  await e.start();
  assert.equal(
    e.clients.length,
    2,
    "a verified finished call permits an explicit fresh start",
  );
});

test("returning to the app automatically resolves unknown retained state without creating or ending a call", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(null, undefined, true);
    client.closed();
  };
  await e.start();
  e.appState("background");
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.deepEqual(e.activeReads, []);
  e.appState("active");
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.equal(e.activeReads.length, 1);
  assert.equal(e.harness.value().conflictUnknown, false);
  assert.equal(e.harness.value().confirmedEnded, true);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("automatic recovery preserves a genuinely active call in another chat", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session(), secondThread);
    client.closed();
  };
  await e.start();
  Object.assign(e.api, {
    accountActive: async () => ({
      session: session(),
      conversation_id: secondThread,
    }),
  });
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.equal(e.statusReads.length, 1);
  assert.equal(e.harness.value().conflict?.id, firstId);
  assert.equal(e.harness.value().conflictThread, secondThread);
  await e.start();
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("automatic status failures and empty discovery preserve the original billing recovery", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(null, undefined, true);
    client.options.onError("Insufficient voice credits", creditRecovery);
    client.closed();
  };
  await e.start();
  e.responses.active = async () => {
    throw new Error("network");
  };
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.equal(e.harness.value().error, "Insufficient voice credits");
  assert.deepEqual(e.harness.value().recovery, creditRecovery);
  e.responses.active = async () => ({ session: null });
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.equal(e.harness.value().confirmedEnded, true);
  assert.equal(e.harness.value().error, "Insufficient voice credits");
  assert.deepEqual(e.harness.value().recovery, creditRecovery);
  e.responses.start = async () => {};
  await e.start();
  assert.equal(e.harness.value().recovery, undefined);
});

test("unmount aborts pending permission, removes app listeners and fences late construction", async (context) => {
  const e = setup(context);
  const waiting = deferred<{ granted: boolean }>();
  e.responses.permission = () => waiting.promise;
  const pending = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  e.harness.unmount();
  assert.equal(e.counts().listeners, 0);
  assert.equal(e.signals[0]!.aborted, true);
  waiting.resolve({ granted: true });
  await pending;
  assert.equal(e.clients.length, 0);
});

test("thread replacement clears captions while retaining the account's exact unresolved call", async (context) => {
  const e = setup(context);
  await e.connect();
  const old = e.clients[0]!;
  old.options.onError("Old thread voice error");
  old.options.onConflict?.(session());
  e.harness.flush();
  const next = e.replaceScope(secondThread);
  assert.equal(old.ends, 1);
  assert.equal(next.connected, false);
  assert.equal(next.starting, false);
  assert.equal(next.snapshot, null);
  assert.equal(next.conflict?.id, firstId);
  assert.equal(next.conflictThread, firstThread);
  assert.equal(next.error, undefined);
  old.options.onSnapshot(snapshot());
  old.options.onError("Late error");
  old.options.onConflict?.(session());
  const final = e.harness.flush();
  assert.equal(final.snapshot, null);
  assert.equal(final.conflict?.id, firstId);
  assert.equal(final.conflictThread, firstThread);
  assert.equal(final.error, undefined);
});

test("account replacement clears existing voice UI and ignores old account captions", async (context) => {
  const e = setup(context);
  await e.connect();
  const old = e.clients[0]!;
  const next = e.replaceScope(firstThread, 2);
  assert.equal(old.ends, 1);
  assert.equal(next.connected, false);
  assert.equal(next.snapshot, null);
  old.options.onSnapshot({
    ...snapshot(),
    captions: [
      {
        id: "old",
        speaker: "user",
        text: "Old account caption",
        sealed: true,
        complete: true,
      },
    ],
  });
  assert.equal(e.harness.flush().snapshot, null);
});

test("conflict metadata survives client closure and blocks new calls until explicit End", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.options.onError("An existing call is active.");
    client.closed();
  };
  const state = await e.start();
  assert.equal(state.conflict?.id, firstId);
  assert.equal(state.starting, false);
  assert.equal(state.connected, false);
  await e.start();
  assert.equal(e.clients.length, 1);
  await e.harness.value().endConflict();
  assert.deepEqual(e.controls, [
    { thread: firstThread, id: firstId, action: "end" },
  ]);
  assert.equal(e.harness.flush().conflict, null);
});

test("explicit recovery ends only the verified other chat and keeps closing blocked", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session(), secondThread);
    client.closed();
  };
  await e.start();
  e.responses.control = async () =>
    session({ state: "closing", end_requested: true });
  await e.harness.value().endConflict();
  let state = e.harness.flush();
  assert.equal(state.conflictThread, secondThread);
  assert.equal(state.conflict?.state, "closing");
  assert.deepEqual(e.controls, [
    { thread: secondThread, id: firstId, action: "end" },
  ]);
  await e.start();
  assert.equal(
    e.clients.length,
    1,
    "accepted End does not yet establish a free slot",
  );
  Object.assign(e.api, {
    accountActive: async () => ({ session: null, conversation_id: null }),
  });
  await e.harness.value().checkConflict();
  state = e.harness.flush();
  assert.equal(state.conflict, null);
  assert.equal(state.error, undefined);
});

test("status discovery preserves a different new call without ending it", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  const staleEnd = e.harness.value().endConflict;
  const staleStart = e.harness.value().start;
  Object.assign(e.api, {
    accountActive: async () => ({
      session: session({ id: secondId }),
      conversation_id: secondThread,
    }),
  });
  await e.harness.value().checkConflict();
  const state = e.harness.flush();
  assert.equal(state.conflict?.id, secondId);
  assert.equal(state.conflictThread, secondThread);
  assert.deepEqual(e.controls, []);
  await staleEnd();
  await staleStart(preferences, "openai_live");
  assert.deepEqual(
    e.controls,
    [],
    "an old End handler cannot target the newly discovered call",
  );
  await e.start();
  assert.equal(e.clients.length, 1);
});

test("unverified account status blocks blind Start and recovers through an explicit check", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(null, undefined, true);
    client.closed();
  };
  await e.start();
  assert.equal(e.harness.value().conflictUnknown, true);
  await e.start();
  assert.equal(e.clients.length, 1);
  Object.assign(e.api, {
    accountActive: async () => ({ session: null, conversation_id: null }),
  });
  await e.harness.value().checkConflict();
  assert.equal(e.harness.flush().conflictUnknown, false);
});

test("a bounded legacy scan can release only a verified prior terminal target for explicit restart", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
      secondThread,
    );
    client.closed();
  };
  await e.start();
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("Voice call discovery unavailable");
    },
  });
  e.responses.status = async () =>
    session({ state: "closed", end_requested: true });
  await e.harness.value().checkConflict();
  const state = e.harness.flush();
  assert.equal(state.conflict, null);
  assert.equal(state.conflictUnknown, false);
  assert.equal(e.statusReads.length, 1);
  assert.equal(e.statusReads[0]!.thread, secondThread);
  assert.equal(e.statusReads[0]!.id, firstId);
  assert.equal(
    e.clients.length,
    1,
    "cleanup never creates a new session automatically",
  );
  assert.deepEqual(e.controls, []);
  e.responses.start = async () => {};
  await e.start();
  assert.equal(
    e.clients.length,
    2,
    "a new explicit Start goes through admission again",
  );
});

for (const result of [
  session({ state: "closing", end_requested: true }),
  session({ id: secondId, state: "closed" }),
]) {
  test(`legacy cleanup keeps a ${result.state} target ${result.id === firstId ? "until terminal" : "with mismatched identity"} blocked`, async (context) => {
    const e = setup(context);
    e.responses.start = async (client) => {
      client.options.onConflict?.(
        session({ state: "closing", end_requested: true }),
      );
      client.closed();
    };
    await e.start();
    Object.assign(e.api, {
      accountActive: async () => {
        throw new Error("Voice call discovery unavailable");
      },
    });
    e.responses.status = async () => result;
    await e.harness.value().checkConflict();
    assert.equal(e.harness.flush().conflict?.id, firstId);
    assert.equal(e.harness.value().conflictUnknown, true);
    await e.start();
    assert.equal(e.clients.length, 1);
    assert.deepEqual(e.controls, []);
  });
}

test("legacy incomplete discovery without an owned target cannot use status to unlock Start", async (context) => {
  const e = setup(context);
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("Voice call discovery unavailable");
    },
  });
  await e.harness.value().checkConflict();
  assert.equal(e.harness.flush().conflictUnknown, true);
  assert.deepEqual(e.statusReads, []);
  await e.start();
  assert.equal(e.clients.length, 0);
});

test("a verified terminal call does not depend on account discovery being available", async (context) => {
  const e = setup(context);
  let accountReads = 0;
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  Object.assign(e.api, {
    accountActive: async () => {
      accountReads++;
      throw new Error("network");
    },
  });
  e.responses.status = async () => session({ state: "closed" });
  assert.equal(await e.harness.value().checkConflict(), true);
  assert.equal(e.harness.flush().conflict, null);
  assert.equal(e.harness.value().conflictUnknown, false);
  assert.equal(accountReads, 0);
  assert.equal(e.statusReads.length, 1);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("a terminal call in another conversation retains an end proof without publishing its snapshot", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
      secondThread,
    );
    client.closed();
  };
  await e.start();
  e.responses.status = async () =>
    session({ state: "closed", end_requested: true });
  assert.equal(await e.harness.value().checkConflict(false), true);
  const state = e.harness.flush();
  assert.equal(state.confirmedEnded, true);
  assert.equal(state.conflict, null);
  assert.equal(state.conflictUnknown, false);
  assert.equal(state.snapshot, null);
  assert.equal(e.statusReads[0]!.thread, secondThread);
  assert.equal(e.statusReads[0]!.id, firstId);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.activeReads, []);
  assert.deepEqual(e.controls, []);
});

for (const invalid of [
  {
    name: "another session",
    value: session({ id: secondId, state: "closed" }),
  },
  {
    name: "an older generation",
    value: session({ generation: 0, state: "closed" }),
  },
  {
    name: "an older control revision",
    value: session({ control_revision: 1, state: "closed" }),
  },
  { name: "an invalid response", value: { state: "closed" } },
]) {
  test(`terminal status from ${invalid.name} cannot release a known call`, async (context) => {
    const e = setup(context);
    e.responses.start = async (client) => {
      client.options.onConflict?.(
        session({ state: "closing", end_requested: true, control_revision: 2 }),
      );
      client.closed();
    };
    await e.start();
    Object.assign(e.api, {
      accountActive: async () => {
        throw new Error("network");
      },
    });
    e.responses.status = async () => invalid.value;
    assert.equal(await e.harness.value().checkConflict(), false);
    const state = e.harness.flush();
    assert.equal(state.conflict?.id, firstId);
    assert.equal(state.conflictUnknown, true);
    assert.equal(state.snapshot, null);
    await e.start();
    assert.equal(e.clients.length, 1);
    assert.deepEqual(e.controls, []);
  });
}

test("failed exact and account status reads keep the owned call blocked", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  e.responses.status = async () => {
    throw new Error("network");
  };
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("network");
    },
  });
  assert.equal(await e.harness.value().checkConflict(), false);
  assert.equal(e.harness.flush().conflict?.id, firstId);
  assert.equal(e.harness.value().conflictUnknown, true);
  assert.equal(e.statusReads.length, 1);
  await e.start();
  assert.equal(e.clients.length, 1);
});

test("automatic status settlement publishes terminal state with the call transcript intact", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  await e.connect();
  const callSnapshot: VoiceSnapshot = {
    ...snapshot(),
    captions: [
      {
        id: "last",
        speaker: "assistant",
        text: "Goodbye",
        sealed: true,
        complete: true,
      },
    ],
    tasks: [
      {
        id: secondId,
        turn_id: "turn",
        state: "completed",
        pending_acknowledgement_ids: [],
        title: "Finished task",
      },
    ],
  };
  e.clients[0]!.options.onSnapshot(callSnapshot);
  e.harness.flush();
  e.clients[0]!.pendingEnd = session({
    state: "closing",
    end_requested: true,
    control_revision: 1,
  });
  assert.equal(await e.harness.value().end(), false);
  e.harness.flush();
  let reads = 0;
  e.responses.status = async () =>
    session({
      state: ++reads === 1 ? "closing" : "closed",
      end_requested: true,
      control_revision: 1,
    });
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("optional discovery is unavailable");
    },
  });
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.equal(e.harness.value().conflict?.state, "closing");
  context.mock.timers.tick(1000);
  await e.harness.settle();
  const state = e.harness.value();
  assert.equal(reads, 2);
  assert.equal(state.conflict, null);
  assert.equal(state.conflictUnknown, false);
  assert.equal(state.snapshot?.session.state, "closed");
  assert.equal(state.snapshot?.session.end_requested, true);
  assert.deepEqual(state.snapshot?.captions, callSnapshot.captions);
  assert.deepEqual(state.snapshot?.tasks, callSnapshot.tasks);
  assert.equal(state.error, undefined);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("automatic settlement retries a transient status failure without another End", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  await e.connect();
  e.clients[0]!.pendingEnd = session({ state: "closing", end_requested: true });
  await e.harness.value().end();
  e.harness.flush();
  let reads = 0;
  e.responses.status = async () => {
    if (++reads === 1) throw new Error("temporary network failure");
    return session({ state: "closed", end_requested: true });
  };
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("optional discovery is unavailable");
    },
  });
  for (let i = 0; i < 2; i++) {
    context.mock.timers.tick(1000);
    await e.harness.settle();
  }
  assert.equal(reads, 2);
  assert.equal(e.harness.value().snapshot?.session.state, "closed");
  assert.equal(e.harness.value().conflict, null);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("automatic settlement retries unchanged failures only within its polling budget", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  e.responses.status = async () => {
    throw new Error("network");
  };
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("network");
    },
  });
  for (let i = 0; i < 10; i++) {
    context.mock.timers.tick(1000);
    await e.harness.settle();
  }
  assert.equal(e.statusReads.length, 6);
  assert.equal(e.harness.value().conflict?.state, "closing");
  assert.equal(e.harness.value().conflictUnknown, true);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("foreground resumes an owned closing call after its polling timer fired in background", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.pendingEnd = session({ state: "closing", end_requested: true });
  e.appState("background");
  await e.harness.settle();
  assert.equal(client.ends, 1);
  assert.equal(e.harness.value().error, undefined);
  client.options.onCleanup?.(null, false);
  assert.equal(e.harness.flush().conflict, null);
  context.mock.timers.tick(10_000);
  await e.harness.settle();
  assert.equal(e.statusReads.length, 0);
  e.responses.status = async () =>
    session({ state: "closed", end_requested: true });
  e.appState("active");
  context.mock.timers.tick(1000);
  await e.harness.settle();
  const state = e.harness.value();
  assert.equal(e.statusReads.length, 1);
  assert.equal(state.snapshot?.session.state, "closed");
  assert.equal(state.conflict, null);
  assert.equal(state.conflictUnknown, false);
  assert.equal(state.error, undefined);
  assert.equal(client.ends, 1);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("verified empty discovery is explicit without inventing a terminal session", async (context) => {
  const e = setup(context);
  await e.connect();
  e.clients[0]!.pendingEnd = session({ state: "closing", end_requested: true });
  await e.harness.value().end();
  e.harness.flush();
  e.responses.status = async () => {
    throw new Error("exact status temporarily unavailable");
  };
  Object.assign(e.api, {
    accountActive: async () => ({ session: null, conversation_id: null }),
  });
  assert.equal(await e.harness.value().checkConflict(false), true);
  const state = e.harness.flush();
  assert.equal(state.confirmedEnded, true);
  assert.equal(state.conflict, null);
  assert.equal(state.snapshot?.session.state, "active");
  assert.equal(state.error, undefined);
  assert.equal(e.clients.length, 1);
  await e.start();
  assert.equal(e.harness.value().confirmedEnded, false);
  assert.equal(e.clients.length, 2);
  assert.equal(e.replaceScope(secondThread, 2).confirmedEnded, false);
});

test("foreground recovery retains the remaining automatic status polling budget", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  e.responses.status = async () => {
    throw new Error("network");
  };
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("network");
    },
  });
  for (let i = 0; i < 2; i++) {
    context.mock.timers.tick(1000);
    await e.harness.settle();
  }
  assert.equal(e.statusReads.length, 2);
  e.appState("background");
  context.mock.timers.tick(10_000);
  await e.harness.settle();
  assert.equal(e.statusReads.length, 2);
  e.appState("active");
  for (let i = 0; i < 10; i++) {
    context.mock.timers.tick(1000);
    await e.harness.settle();
  }
  assert.equal(e.statusReads.length, 6);
  e.appState("inactive");
  e.appState("active");
  for (let i = 0; i < 2; i++) {
    context.mock.timers.tick(1000);
    await e.harness.settle();
  }
  assert.equal(e.statusReads.length, 6);
  assert.equal(e.harness.value().conflict?.state, "closing");
  assert.equal(e.clients.length, 1);
  assert.equal(e.clients[0]!.ends, 0);
  assert.deepEqual(e.controls, []);
});

test("late terminal status after a scope change cannot clear another account's recovery", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  Object.assign(e.api, {
    accountActive: async () => {
      throw new Error("Voice call discovery unavailable");
    },
  });
  const waiting = deferred<unknown>();
  e.responses.status = async () => waiting.promise;
  const checking = e.harness.value().checkConflict();
  await e.harness.settle();
  assert.equal(e.statusReads.length, 1);
  e.replaceScope(secondThread, 2);
  assert.equal(e.statusReads[0]!.signal?.aborted, true);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session({ id: secondId }), secondThread);
    client.closed();
  };
  await e.start();
  waiting.resolve(session({ state: "closed" }));
  await checking;
  assert.equal(e.harness.flush().conflict?.id, secondId);
});

test("an owned End that failed remotely remains available for recovery", async (context) => {
  const e = setup(context);
  await e.connect();
  e.clients[0]!.pendingEnd = session();
  assert.equal(await e.harness.value().end(), false);
  const state = e.harness.flush();
  assert.equal(state.connected, false);
  assert.equal(state.conflict?.id, firstId);
  assert.equal(state.conflictThread, firstThread);
  assert.equal(e.clients[0]!.ends, 1);
});

test("accepted End stays inspectable across unmount until terminal cleanup is confirmed", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.pendingEnd = session({ state: "closing", end_requested: true });
  assert.equal(await e.harness.value().end(), false);
  e.harness.unmount();
  const restored = e.remount();
  assert.equal(restored.conflict?.id, firstId);
  assert.equal(restored.conflictThread, firstThread);
  assert.equal(restored.connected, false);
  await e.start();
  assert.equal(e.clients.length, 1);
  client.pendingEnd = undefined;
  client.options.onCleanup?.(null, false);
  assert.equal(e.harness.flush().conflict, null);
  assert.equal(await e.harness.value().end(), true);
});

test("an unresolved create survives leaving voice and late ownership is recovered without another create", async (context) => {
  const e = setup(context);
  const waiting = deferred<void>();
  e.responses.start = async (client) => {
    client.pendingStart = true;
    return waiting.promise;
  };
  const starting = e.harness.value().start(preferences, "openai_live");
  await e.harness.settle();
  const client = e.clients[0]!;
  assert.equal(await e.harness.value().end(), false);
  e.harness.unmount();
  assert.equal(e.remount().conflictUnknown, true);
  assert.equal(await e.harness.value().checkConflict(), false);
  assert.deepEqual(
    e.activeReads,
    [],
    "an empty status cannot release an in-flight create",
  );
  await e.start();
  assert.equal(e.clients.length, 1);
  client.pendingStart = false;
  client.options.onCleanup?.(
    session({ state: "closing", end_requested: true }),
    false,
    firstThread,
  );
  waiting.resolve();
  await starting;
  const restored = e.harness.flush();
  assert.equal(restored.conflictUnknown, false);
  assert.equal(restored.conflict?.id, firstId);
  assert.equal(restored.conflictThread, firstThread);
  assert.deepEqual(e.controls, []);
});

test("only a terminal conflict End or verified empty status permits closing the surface", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  e.responses.control = async () =>
    session({ state: "closing", end_requested: true });
  assert.equal(await e.harness.value().endConflict(), false);
  e.harness.flush();
  e.responses.active = async () => ({
    session: session({ state: "closing", end_requested: true }),
  });
  assert.equal(await e.harness.value().checkConflict(), false);
  e.responses.active = async () => ({ session: null });
  assert.equal(await e.harness.value().checkConflict(), true);
});

test("automatic failure with End still pending blocks a retained Start handler immediately", async (context) => {
  const e = setup(context);
  const staleStart = e.harness.value().start;
  await e.connect();
  const client = e.clients[0]!;
  client.pendingEnd = session();
  client.options.onError("Voice connection lost.");
  client.closed();
  const state = e.harness.flush();
  assert.equal(state.connected, false);
  assert.equal(state.conflict?.id, firstId);
  await staleStart(preferences, "openai_live");
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("closing status polling is bounded and never repeats End or creation", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  let reads = 0;
  Object.assign(e.api, {
    accountActive: async () => {
      reads++;
      return {
        session: session({ state: "closing", end_requested: true }),
        conversation_id: firstThread,
      };
    },
  });
  e.responses.start = async (client) => {
    client.options.onConflict?.(
      session({ state: "closing", end_requested: true }),
    );
    client.closed();
  };
  await e.start();
  for (let i = 0; i < 10; i++) {
    context.mock.timers.tick(1000);
    await e.harness.settle();
  }
  assert.equal(reads, 6);
  assert.equal(e.harness.value().conflict?.state, "closing");
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.controls, []);
});

test("a late status check cannot replace recovery after an account change", async (context) => {
  const e = setup(context);
  const waiting = deferred<unknown>();
  Object.assign(e.api, { accountActive: async () => waiting.promise });
  const checking = e.harness.value().checkConflict();
  e.replaceScope(secondThread, 2);
  waiting.resolve({ session: session(), conversation_id: firstThread });
  await checking;
  assert.equal(e.harness.flush().conflict, null);
  assert.equal(e.harness.value().checkingConflict, false);
});

test("billing recovery survives failed client closure and clears as soon as a new explicit attempt starts", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onError("Safe billing diagnostic", creditRecovery);
    client.closed();
  };
  const failed = await e.start();
  assert.equal(failed.error, "Safe billing diagnostic");
  assert.deepEqual(failed.recovery, creditRecovery);
  assert.equal(failed.starting, false);
  assert.equal(failed.connected, false);
  const waiting = deferred<any>();
  e.responses.options = () => waiting.promise;
  const next = e.harness.value().start(preferences, "openai_live");
  const starting = e.harness.flush();
  assert.equal(starting.error, undefined);
  assert.equal(starting.recovery, undefined);
  assert.equal(starting.starting, true);
  waiting.resolve({ options: [] });
  await next;
  assert.equal(e.harness.flush().recovery, undefined);
});

test("another error replaces billing recovery instead of retaining unrelated billing instructions", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.options.onError("Safe billing diagnostic", creditRecovery);
  assert.deepEqual(e.harness.flush().recovery, creditRecovery);
  client.options.onError("Tap Resume audio to hear the assistant.");
  const state = e.harness.flush();
  assert.equal(state.error, "Tap Resume audio to hear the assistant.");
  assert.equal(state.recovery, undefined);
  client.options.onError("Safe billing diagnostic", creditRecovery);
  e.harness.flush();
  assert.equal(e.appState("background").recovery, undefined);
});

test("opening another app while idle preserves billing recovery and never claims an idle call ended", async (context) => {
  const e = setup(context);
  assert.equal(e.appState("inactive").error, undefined);
  assert.equal(e.appState("background").error, undefined);
  e.appState("active");
  e.responses.start = async (client) => {
    client.options.onError("Safe billing diagnostic", creditRecovery);
    client.closed();
  };
  await e.start();
  for (const appState of ["inactive", "background", "active"]) {
    const state = e.appState(appState);
    assert.equal(state.error, "Safe billing diagnostic");
    assert.deepEqual(state.recovery, creditRecovery);
    assert.equal(state.connected, false);
  }
  assert.equal(e.clients.length, 1);
});

for (const scope of ["thread", "account"] as const) {
  test(`${scope} replacement clears billing recovery and rejects stale recovery callbacks`, async (context) => {
    const e = setup(context);
    await e.connect();
    const old = e.clients[0]!;
    old.options.onError("Old billing diagnostic", creditRecovery);
    assert.deepEqual(e.harness.flush().recovery, creditRecovery);
    const next =
      scope === "thread"
        ? e.replaceScope(secondThread)
        : e.replaceScope(firstThread, 2);
    assert.equal(next.error, undefined);
    assert.equal(next.recovery, undefined);
    old.options.onError("Late billing diagnostic", creditRecovery);
    assert.equal(e.harness.flush().recovery, undefined);
  });
}

test("duplicate End existing call presses must issue one durable control", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  const waiting = deferred<unknown>();
  e.responses.control = () => waiting.promise;
  const endConflict = e.harness.value().endConflict;
  const first = endConflict();
  const second = endConflict();
  assert.equal(e.controls.length, 1);
  waiting.resolve(session({ state: "closed" }));
  await Promise.all([first, second]);
});

test("late End conflict success cannot clear a different thread's newly observed call", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  const waiting = deferred<unknown>();
  e.responses.control = () => waiting.promise;
  const oldEnd = e.harness.value().endConflict();
  e.replaceScope(secondThread);
  Object.assign(e.api, {
    accountActive: async () => ({
      session: session({ id: secondId }),
      conversation_id: secondThread,
    }),
  });
  await e.harness.value().checkConflict();
  assert.equal(e.harness.flush().conflict?.id, secondId);
  waiting.resolve(session({ state: "closed" }));
  await oldEnd;
  assert.equal(e.harness.flush().conflict?.id, secondId);
});

test("failed End conflict refreshes only the active call in the same scope", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  e.responses.control = async () => {
    throw new Error("network");
  };
  e.responses.active = async () => ({ session: session({ id: secondId }) });
  await e.harness.value().endConflict();
  assert.deepEqual(e.activeReads, [firstThread]);
  assert.equal(e.harness.flush().conflict?.id, secondId);
  assert.equal(e.harness.value().endingConflict, false);
});

test("late End conflict failure after account switch cannot install stale error or active metadata", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  const waiting = deferred<unknown>();
  e.responses.control = () => waiting.promise;
  const ending = e.harness.value().endConflict();
  e.replaceScope(secondThread, 2);
  waiting.reject(new Error("network"));
  await ending;
  const state = e.harness.flush();
  assert.equal(state.error, undefined);
  assert.equal(state.conflict, null);
  assert.equal(state.endingConflict, false);
  assert.deepEqual(e.activeReads, []);
});

test("backgrounding during End existing call cancels its busy UI and allows explicit retry", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.closed();
  };
  await e.start();
  const waiting = deferred<unknown>();
  e.responses.control = () => waiting.promise;
  const oldEnd = e.harness.value().endConflict();
  e.harness.flush();
  assert.equal(e.harness.value().endingConflict, true);
  e.appState("background");
  e.appState("active");
  waiting.resolve(session({ state: "closed" }));
  await oldEnd;
  assert.equal(e.harness.flush().endingConflict, false);
  e.responses.control = async () => session({ state: "closed" });
  await e.harness.value().endConflict();
  assert.equal(
    e.controls.length,
    2,
    "the stale request cannot hold the new user action busy forever",
  );
});

test("replacing scope clears a pending refresh handle so the next thread can refresh", async (context) => {
  const e = setup(context);
  await e.connect();
  e.replaceScope(secondThread);
  await e.connect();
  await new Promise<void>((resolve) => setTimeout(resolve, 1050));
  assert.equal(
    e.counts().refreshCalls,
    1,
    "only the new thread's committed refresh should run",
  );
});

test("End voice leaves accepted work running; Stop delegates only the selected request", async (context) => {
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  await e.harness.value().stopTask(secondId);
  assert.deepEqual(client.stops, [secondId]);
  await e.harness.value().end();
  assert.equal(client.ends, 1);
  assert.deepEqual(client.stops, [secondId]);
  assert.equal(e.harness.flush().connected, false);
});

test("holding UI follows accepted presses and clears on blocked or recovered snapshots and local mute", async (context) => {
  const e = setup(context);
  await e.connect();
  e.responses.hold = () => false;
  e.harness.value().hold(true);
  assert.equal(
    e.harness.flush().holding,
    false,
    "a refused controller press is not shown as recording",
  );
  e.responses.hold = (pressed) => pressed;
  for (const blocked of [
    { muted: true },
    { input_muted: true },
    { end_requested: true },
    { state: "closing" as const },
    { generation: 2 },
  ]) {
    e.clients[0]!.options.onSnapshot(snapshot());
    e.harness.flush();
    e.harness.value().hold(true);
    assert.equal(e.harness.flush().holding, true);
    e.clients[0]!.options.onSnapshot(snapshot(blocked));
    assert.equal(e.harness.flush().holding, false);
    e.clients[0]!.options.onSnapshot(snapshot({ generation: 3 }));
    assert.equal(
      e.harness.flush().holding,
      false,
      "recovery cannot restore the old pressed UI",
    );
  }
  e.harness.value().hold(true);
  e.harness.flush();
  await e.harness.value().mute(true);
  assert.equal(e.harness.flush().holding, false);
});

test("after End, Stop still cancels the verified queued task without restarting voice or cancelling other work", async (context) => {
  const e = setup(context);
  await e.connect();
  e.clients[0]!.options.onSnapshot({
    ...snapshot(),
    tasks: [
      {
        id: secondId,
        turn_id: "turn",
        state: "queued",
        pending_acknowledgement_ids: [],
      },
    ],
  });
  e.harness.flush();
  await e.harness.value().end();
  assert.equal(e.cancellations.length, 0, "End itself leaves tasks running");
  e.harness.flush();
  const waiting = deferred<void>();
  e.responses.cancel = () => waiting.promise;
  const stopTask = e.harness.value().stopTask;
  const first = stopTask(secondId);
  const duplicate = stopTask(secondId);
  assert.equal(e.cancellations.length, 1);
  assert.equal(e.cancellations[0]!.thread, firstThread);
  assert.equal(e.cancellations[0]!.id, secondId);
  waiting.resolve();
  await Promise.all([first, duplicate]);
  assert.equal(e.clients.length, 1);
  assert.deepEqual(e.clients[0]!.stops, [], "the closed client is not reused");
});

test("post-call Stop rejects unverified or completed requests and stale account or thread handlers", async (context) => {
  const e = setup(context);
  await e.connect();
  e.clients[0]!.options.onSnapshot({
    ...snapshot(),
    tasks: [
      {
        id: secondId,
        turn_id: "turn",
        state: "completed",
        pending_acknowledgement_ids: [],
      },
    ],
  });
  e.clients[0]!.closed();
  e.harness.flush();
  await e.harness.value().stopTask(firstId);
  await e.harness.value().stopTask(secondId);
  assert.equal(e.cancellations.length, 0);
  e.clients[0]!.options.onSnapshot({
    ...snapshot(),
    tasks: [
      {
        id: secondId,
        turn_id: "turn",
        state: "claimed",
        pending_acknowledgement_ids: [],
      },
    ],
  });
  e.harness.flush();
  const oldStop = e.harness.value().stopTask;
  e.replaceScope(secondThread, 2);
  await oldStop(secondId);
  assert.equal(e.cancellations.length, 0);
});

test("post-call task cancellation is aborted on scope exit and late errors cannot enter the next chat", async (context) => {
  const e = setup(context);
  await e.connect();
  e.clients[0]!.options.onSnapshot({
    ...snapshot(),
    tasks: [
      {
        id: secondId,
        turn_id: "turn",
        state: "claimed",
        pending_acknowledgement_ids: [],
      },
    ],
  });
  e.clients[0]!.closed();
  e.harness.flush();
  const waiting = deferred<void>();
  e.responses.cancel = () => waiting.promise;
  const cancelling = e.harness.value().stopTask(secondId);
  e.replaceScope(secondThread, 2);
  assert.equal(e.cancellations[0]!.signal.aborted, true);
  waiting.reject(new Error("old network error"));
  await cancelling;
  assert.equal(e.harness.flush().error, undefined);
});

test("the call clock waits for client readiness, survives updates and resets for a new attempt", async (context) => {
  const f = setup(context);
  await f.start();
  assert.equal(f.harness.value().connectedAt, undefined);
  f.clients.at(-1)!.options.onSnapshot(snapshot({ state: "starting" }));
  assert.equal(f.harness.flush().connectedAt, undefined);
  const before = Date.now();
  f.clients.at(-1)!.options.onSnapshot(snapshot());
  assert.equal(f.harness.flush().connectedAt, undefined);
  assert.equal(f.harness.value().connected, false);
  f.clients.at(-1)!.options.onReady?.();
  const connectedAt = f.harness.flush().connectedAt!;
  assert.ok(connectedAt >= before && connectedAt <= Date.now());
  f.clients.at(-1)!.options.onSnapshot(snapshot({ observed_seconds: 8 }));
  assert.equal(f.harness.flush().connectedAt, connectedAt);
  f.clients.at(-1)!.closed();
  const ended = f.harness.flush().disconnectedAt!;
  assert.ok(ended >= connectedAt);
  assert.equal(f.harness.value().connectedAt, connectedAt);
  await f.start();
  assert.equal(f.harness.value().connectedAt, undefined);
  assert.equal(f.harness.value().disconnectedAt, undefined);
});

test("local End leaves media teardown to the client and blocks recovery reads during its drain", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const e = setup(context);
  await e.connect();
  const client = e.clients[0]!;
  client.pendingEnd = session({ state: "closing", end_requested: true });
  const drained = deferred<void>();
  e.responses.end = async (closing) => {
    await drained.promise;
    closing.options.media.close();
    closing.options.onCleanup?.(closing.pendingEnd ?? null, false, firstThread);
  };
  const ending = e.harness.value().end();
  assert.equal(e.harness.flush().inputEnabled, false);
  assert.equal(
    e.media[0]!.closed,
    0,
    "hook must not cut the client drain short",
  );
  context.mock.timers.tick(1000);
  await e.harness.settle();
  assert.deepEqual(e.statusReads, [], "polling cannot steal cleanup ownership");
  assert.equal(await e.harness.value().checkConflict(), false);
  const restarting = e.start();
  assert.equal(e.clients.length, 1);
  drained.resolve();
  assert.equal(await ending, false);
  await restarting;
  assert.equal(e.media[0]!.closed, 1);
  assert.equal(e.harness.flush().conflict?.id, firstId);
});

for (const stage of [
  "exact status",
  "account discovery",
  "End control",
  "End fallback discovery",
] as const) {
  test(`${stage} has an eight-second bound even if its request ignores abort`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    const e = setup(context);
    const closing = session({ state: "closing", end_requested: true });
    e.responses.start = async (client) => {
      client.options.onConflict?.(closing);
      client.closed();
    };
    await e.start();
    const waiting = deferred<unknown>();
    e.responses.active = async () => ({ session: closing });
    if (stage === "exact status") e.responses.status = () => waiting.promise;
    else if (stage === "account discovery")
      e.responses.active = () => waiting.promise;
    else if (stage === "End control")
      e.responses.control = () => waiting.promise;
    else {
      e.responses.control = async () => {
        throw new Error("network");
      };
      e.responses.active = () => waiting.promise;
    }
    const isEnd = stage.startsWith("End");
    const operation = isEnd
      ? e.harness.value().endConflict()
      : e.harness.value().checkConflict();
    await e.harness.settle();
    assert.equal(
      isEnd
        ? e.harness.value().endingConflict
        : e.harness.value().checkingConflict,
      true,
    );
    context.mock.timers.tick(7999);
    await e.harness.settle();
    assert.equal(
      isEnd
        ? e.harness.value().endingConflict
        : e.harness.value().checkingConflict,
      true,
    );
    context.mock.timers.tick(1);
    assert.equal(await operation, false);
    const state = e.harness.flush();
    assert.equal(state.endingConflict, false);
    assert.equal(state.checkingConflict, false);
    assert.equal(state.conflict?.id, firstId);
    assert.match(state.error ?? "", /could not be (verified|confirmed)/);
    const signal = isEnd ? e.controlSignals[0] : e.statusReads[0]!.signal;
    assert.equal(signal?.aborted, true);
    waiting.resolve(
      stage.includes("discovery")
        ? { session: null }
        : session({ state: "closed" }),
    );
    await e.harness.settle();
    assert.equal(
      e.harness.value().conflict?.id,
      firstId,
      "late success cannot clear retained recovery",
    );
    await e.start();
    assert.equal(
      e.clients.length,
      1,
      "unverified state never starts another call",
    );
  });
}

test("verified closing discovery replaces the stale another-call diagnostic", async (context) => {
  const e = setup(context);
  e.responses.start = async (client) => {
    client.options.onConflict?.(session());
    client.options.onError(
      "Another voice call is active",
      undefined,
      "conflict",
    );
    client.closed();
  };
  await e.start();
  e.responses.active = async () => ({
    session: session({ state: "closing", end_requested: true }),
  });
  await e.harness.value().checkConflict(false);
  assert.equal(e.harness.flush().conflict?.state, "closing");
  assert.equal(e.harness.value().error, undefined);
  assert.equal(e.controls.length, 0);
});

for (const cancel of ["end", "background"] as const) {
  test(`a later ${cancel} cancels an explicit start waiting on an older drain`, async (context) => {
    const e = setup(context);
    await e.connect();
    const drained = deferred<void>();
    e.responses.end = () => drained.promise;
    const ending = e.harness.value().end();
    const starting = e.harness.value().start(preferences, "openai_live");
    assert.equal(e.clients.length, 1);
    const secondEnd = cancel === "end" ? e.harness.value().end() : undefined;
    if (cancel === "background") {
      e.appState("background");
      e.appState("active");
    }
    drained.resolve();
    await Promise.all([ending, starting, secondEnd]);
    assert.equal(
      e.clients.length,
      1,
      "a later stop cancels the waiting intent even after recovery clears",
    );
    assert.equal(e.harness.flush().connected, false);
  });
}
