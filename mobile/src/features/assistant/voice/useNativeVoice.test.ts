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
  const cancellations: { thread: string; id: string; signal: AbortSignal }[] =
    [];
  const activeReads: string[] = [];
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
    control: async (): Promise<unknown> => session({ state: "closed" }),
    active: async (): Promise<any> => ({ session: null }),
    cancel: async (): Promise<void> => {},
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
    ) => {
      controls.push({ thread: selectedThread, id, action: input.action });
      return responses.control();
    },
    active: async (selectedThread: string) => {
      activeReads.push(selectedThread);
      return responses.active();
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
      this.options.onClosed?.();
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
  harness.mount(() =>
    loaded.useNativeVoice(thread, environment, () => {
      refreshCalls++;
    }),
  );
  context.after(() => harness.unmount());
  const start = async () => {
    await harness.value().start(preferences, "openai_live");
    return harness.flush();
  };
  const connect = async () => {
    await start();
    clients.at(-1)!.options.onSnapshot(snapshot());
    return harness.flush();
  };
  return {
    harness,
    clients,
    media,
    controls,
    cancellations,
    activeReads,
    available,
    responses,
    native,
    signals,
    start,
    connect,
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
  client.options.onSnapshot(snapshot({ generation: 2 }));
  assert.equal(e.harness.flush().connected, false);
  assert.equal(e.clients.length, 1);
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

test("thread replacement closes the old client and clears the old thread's visible session", async (context) => {
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
  assert.equal(next.conflict, null);
  assert.equal(next.error, undefined);
  old.options.onSnapshot(snapshot());
  old.options.onError("Late error");
  old.options.onConflict?.(session());
  const final = e.harness.flush();
  assert.equal(final.snapshot, null);
  assert.equal(final.conflict, null);
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
  e.responses.start = async (client) => {
    client.options.onConflict?.(session({ id: secondId }));
    client.closed();
  };
  await e.start();
  assert.equal(e.harness.value().conflict?.id, secondId);
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
