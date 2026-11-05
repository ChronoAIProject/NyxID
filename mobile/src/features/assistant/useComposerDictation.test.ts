import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { useComposerDictation } from "./useComposerDictation";

type Listener = (event?: any) => void;
type Permission = { granted: boolean; restricted?: boolean };

/** The native recognizer: one shared session with an event emitter. */
function recognizer(events: string[]) {
  const listeners = new Map<string, Set<Listener>>();
  let permission = deferred<Permission>();
  const module = {
    onDevice: true,
    options: undefined as Record<string, unknown> | undefined,
    requestPermissionsAsync: () => {
      events.push("permission");
      return permission.promise;
    },
    getSupportedLocales: async () => ({
      locales: ["en-GB", "en-US"],
      installedLocales: [],
    }),
    supportsOnDeviceRecognition: () => module.onDevice,
    addListener: (name: string, listener: Listener) => {
      const set = listeners.get(name) ?? new Set();
      listeners.set(name, set);
      set.add(listener);
      return { remove: () => set.delete(listener) };
    },
    start: (options: Record<string, unknown>) => {
      events.push("start");
      module.options = options;
    },
    stop: () => events.push("stop"),
    abort: () => events.push("abort"),
    emit: (name: string, event?: unknown) => {
      for (const listener of [...(listeners.get(name) ?? [])]) listener(event);
    },
    listening: () =>
      [...listeners.entries()]
        .filter(([, set]) => set.size)
        .map(([name]) => name)
        .sort(),
    grant: (value: Permission) => permission.resolve(value),
    resetPermission: () => {
      permission = deferred<Permission>();
    },
  };
  return module;
}

function setup({
  chosen,
  continuous = true,
}: { chosen?: string; continuous?: boolean } = {}) {
  let state = "active",
    focused = true,
    sheetOpen = false,
    epoch = 1;
  const appListeners = new Set<(state: string) => void>();
  const events: string[] = [],
    inserted: string[] = [];
  let resolveForeground: (() => void) | undefined;
  const native = recognizer(events);
  const locales: [string | undefined, readonly string[]][] = [];
  const props = {
    scope: "agent:chat:0",
    continuous,
    enabled: true,
    onInsert: (text: string) => {
      inserted.push(text);
    },
  };
  const h = hookHarness<ReturnType<typeof useComposerDictation>>({
    "react-native": {
      Linking: {
        openSettings: async () => {
          events.push("settings");
        },
      },
      AppState: {
        get currentState() {
          return state;
        },
        addEventListener: (_: string, callback: (state: string) => void) => {
          appListeners.add(callback);
          return { remove: () => appListeners.delete(callback) };
        },
      },
    },
    "expo-speech-recognition": { ExpoSpeechRecognitionModule: native },
    "@react-navigation/native": { useIsFocused: () => focused },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/accessibility/workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({ sheetOpen }),
    },
    "./voice/dictationLanguage": {
      savedDictationLanguage: async () => chosen,
    },
    "./voice/dictationLocale": {
      dictationLocale: (phone: string | undefined, list: readonly string[]) => {
        locales.push([phone, list]);
        return "en-GB";
      },
    },
    "./voice/voicePermission": {
      waitForVoiceForeground: async (_app: unknown, signal: AbortSignal) => {
        if (state === "active") return;
        await new Promise<void>((resolve, reject) => {
          resolveForeground = resolve;
          signal.addEventListener(
            "abort",
            () => reject(new Error("cancelled")),
            { once: true },
          );
        });
      },
    },
  });
  const module = h.load(
    fileURLToPath(new URL("./useComposerDictation.ts", import.meta.url)),
  );
  h.mount(() =>
    (module.useComposerDictation as typeof useComposerDictation)(props),
  );
  const emit = (name: string, event?: unknown) => {
    native.emit(name, event);
    h.flush();
  };
  const result = (transcript: string, isFinal = false) =>
    emit("result", { isFinal, results: [{ transcript, confidence: 1 }] });
  return {
    h,
    props,
    events,
    inserted,
    native,
    locales,
    emit,
    result,
    append: module.appendDictation as (draft: string, text: string) => string,
    /** Permission granted and the recognizer reported start. */
    begin: async () => {
      const pending = h.value().start();
      native.grant({ granted: true });
      await pending;
      await h.settle();
      emit("start", null);
    },
    change: (what: string) => {
      if (what === "account") epoch++;
      if (what === "chat") props.scope = "other";
      if (what === "disabled") props.enabled = false;
      if (what === "focus") focused = false;
      if (what === "sheet") sheetOpen = true;
      h.render();
    },
    app: (next: string) => {
      state = next;
      for (const cb of [...appListeners]) cb(next);
      if (next === "active") resolveForeground?.();
      h.flush();
    },
  };
}

test("native dictation starts Apple recognition in the phone's language without any NyxID call", async () => {
  const x = setup();
  await x.begin();
  assert.deepEqual(x.events, ["permission", "start"], "no server draft");
  assert.deepEqual(JSON.parse(JSON.stringify(x.native.options)), {
    lang: "en-GB",
    interimResults: true,
    continuous: true,
    addsPunctuation: true,
    maxAlternatives: 1,
    iosTaskHint: "dictation",
    requiresOnDeviceRecognition: true,
    volumeChangeEventOptions: { enabled: true, intervalMillis: 90 },
  });
  assert.equal(typeof x.locales[0]![0], "string");
  assert.deepEqual(x.locales[0]![1], ["en-GB", "en-US"]);
  assert.equal(x.h.value().phase, "listening");
  assert.equal(x.h.value().recording, true);
  x.h.unmount();
});

test("on-device recognition is only required when the iPhone supports it", async () => {
  const x = setup();
  x.native.onDevice = false;
  await x.begin();
  assert.equal(x.native.options!.requiresOnDeviceRecognition, false);
  x.h.unmount();
});

test("interim words are replaced, final segments append, and finishing inserts once into the draft", async () => {
  const x = setup();
  await x.begin();
  x.result("Book a");
  assert.deepEqual(
    [x.h.value().final, x.h.value().interim, x.h.value().text],
    ["", "Book a", "Book a"],
  );
  x.result("Book a table");
  x.result("Book a table", true);
  x.result(" for two");
  assert.deepEqual(
    [x.h.value().final, x.h.value().interim, x.h.value().text],
    ["Book a table", "for two", "Book a table for two"],
  );
  assert.deepEqual(x.inserted, []);
  x.h.value().stop();
  x.h.flush();
  assert.equal(x.h.value().phase, "stopping");
  assert.equal(x.events.at(-1), "stop");
  x.h.value().stop();
  x.h.flush();
  assert.equal(x.events.filter((e) => e === "stop").length, 1);
  x.result(" for two", true);
  x.emit("end", null);
  x.emit("end", null);
  x.h.flush();
  assert.deepEqual(x.native.listening(), [], "no recognizer listeners remain");
  assert.deepEqual(x.inserted, ["Book a table for two"]);
  assert.equal(x.h.value().phase, "idle");
  assert.equal(x.h.value().text, "", "the words now live in the draft");
  assert.ok(!x.events.includes("abort"), "a finished session needs no abort");
  assert.equal(x.append("Existing\n", "Hello"), "Existing\nHello");
  assert.equal(x.append("Existing", "Hello"), "Existing Hello");
  x.h.unmount();
});

test("the meter receives normalised native volume while listening", async () => {
  const x = setup();
  await x.begin();
  const levels: number[] = [];
  const unsubscribe = x.h.value().subscribeLevel((level) => levels.push(level));
  x.emit("volumechange", { value: -2 });
  x.emit("volumechange", { value: 5 });
  x.emit("volumechange", { value: 11 });
  unsubscribe();
  x.emit("volumechange", { value: 5 });
  assert.deepEqual(levels, [0, 0.5, 1]);
  x.h.unmount();
});

test("discard aborts recognition and ignores late results without inserting", async () => {
  const x = setup();
  await x.begin();
  x.result("Private");
  x.h.value().discard();
  x.h.flush();
  assert.equal(x.events.at(-1), "abort");
  x.result("Late", true);
  x.emit("end", null);
  assert.equal(x.h.value().phase, "idle");
  assert.equal(x.h.value().text, "");
  assert.deepEqual(x.inserted, []);
  x.h.unmount();
});

test("account/chat/access/focus/sheet changes abort capture and never insert stale words", async () => {
  for (const change of ["account", "chat", "disabled", "focus", "sheet"]) {
    const x = setup();
    await x.begin();
    x.result("Old");
    x.change(change);
    assert.equal(x.events.at(-1), "abort", change);
    x.result("Late", true);
    x.emit("end", null);
    assert.equal(x.h.value().phase, "idle", change);
    assert.deepEqual(x.inserted, []);
    x.h.unmount();
  }
});

test("unmount and background abort a live recognizer so the microphone never stays on", async () => {
  const x = setup();
  await x.begin();
  x.h.unmount();
  assert.equal(x.events.at(-1), "abort");
  const y = setup();
  await y.begin();
  y.result("Private");
  y.app("background");
  assert.equal(y.events.at(-1), "abort");
  assert.equal(y.h.value().text, "");
  assert.equal(y.h.value().active, false);
  y.h.unmount();
});

test("late permission cannot start recognition after cancellation or unmount", async () => {
  for (const change of [
    "account",
    "chat",
    "disabled",
    "focus",
    "sheet",
    "unmount",
  ]) {
    const x = setup();
    const pending = x.h.value().start();
    if (change === "unmount") x.h.unmount();
    else x.change(change);
    x.native.grant({ granted: true });
    await pending;
    await x.h.settle();
    assert.deepEqual(x.events, ["permission"], change);
    if (change !== "unmount") x.h.unmount();
  }
});

test("permission denial links to Settings and never starts recognition", async () => {
  for (const restricted of [false, true]) {
    const x = setup();
    const pending = x.h.value().start();
    x.native.grant({ granted: false, restricted });
    await pending;
    await x.h.settle();
    assert.match(
      x.h.value().error!,
      restricted
        ? /restricted.*Screen Time/
        : /microphone and speech recognition.*Settings/,
    );
    assert.equal(x.h.value().permissionDenied, true);
    assert.equal(x.h.value().active, false);
    x.h.value().openSettings();
    assert.deepEqual(x.events, ["permission", "settings"]);
    x.h.unmount();
  }
});

test("the permission prompt's inactive state waits for foreground; background during capture aborts", async () => {
  const x = setup();
  const pending = x.h.value().start();
  x.app("inactive");
  x.native.grant({ granted: true });
  await x.h.settle();
  assert.deepEqual(x.events, ["permission"]);
  x.app("active");
  await pending;
  await x.h.settle();
  assert.deepEqual(x.events, ["permission", "start"]);
  x.emit("start", null);
  x.app("inactive");
  assert.equal(x.events.at(-1), "abort", "Control Center ends capture too");
  assert.equal(x.h.value().active, false);
  x.h.unmount();
});

test("hold release before the microphone opens cancels without recognizing", async () => {
  const x = setup();
  const pending = x.h.value().start();
  x.h.value().stop();
  x.native.grant({ granted: true });
  await pending;
  await x.h.settle();
  assert.deepEqual(x.events, ["permission"]);
  assert.equal(x.h.value().phase, "idle");
  x.h.unmount();
  const y = setup();
  const started = y.h.value().start();
  y.native.grant({ granted: true });
  await started;
  await y.h.settle();
  assert.equal(y.h.value().phase, "connecting");
  y.h.value().stop();
  y.h.flush();
  assert.equal(y.events.at(-1), "abort", "started but not yet listening");
  assert.equal(y.h.value().phase, "idle");
  y.h.unmount();
});

test("a new attempt waits for an aborted session's end before listening", async () => {
  const x = setup();
  await x.begin();
  x.h.value().discard();
  x.h.flush();
  x.native.resetPermission();
  const pending = x.h.value().start();
  x.native.grant({ granted: true });
  await x.h.settle();
  assert.deepEqual(x.events, ["permission", "start", "abort", "permission"]);
  x.emit("end", null); // the aborted session's end
  await pending;
  await x.h.settle();
  assert.equal(x.events.at(-1), "start");
  assert.equal(
    x.h.value().phase,
    "connecting",
    "the stale end did not settle it",
  );
  x.emit("start", null);
  assert.equal(x.h.value().phase, "listening");
  x.h.unmount();
});

test("recognizer errors explain themselves; text heard before a failure still joins the draft", async () => {
  const cases: [string, RegExp][] = [
    ["no-speech", /No speech was detected/],
    ["network", /connection/],
    ["language-not-supported", /iPhone's language/],
    ["service-not-allowed", /Dictation is on in Settings/],
    ["audio-capture", /microphone couldn't record/],
  ];
  for (const [code, message] of cases) {
    const x = setup();
    await x.begin();
    x.emit("error", { error: code, message: "native detail" });
    x.emit("end", null);
    assert.equal(x.h.value().phase, "idle", code);
    assert.match(x.h.value().error!, message, code);
    assert.equal(x.h.value().permissionDenied, false, code);
    x.h.unmount();
  }
  const denied = setup();
  await denied.begin();
  denied.emit("error", { error: "not-allowed", message: "" });
  denied.emit("end", null);
  assert.equal(denied.h.value().permissionDenied, true);
  assert.match(denied.h.value().error!, /Settings/);
  denied.h.unmount();
  const silent = setup();
  await silent.begin();
  silent.emit("nomatch", null);
  silent.emit("end", null);
  assert.match(silent.h.value().error!, /No speech/);
  silent.h.unmount();
  const kept = setup();
  await kept.begin();
  kept.result("Keep me");
  kept.emit("error", { error: "interrupted", message: "" });
  kept.emit("end", null);
  assert.equal(kept.h.value().phase, "idle");
  assert.match(kept.h.value().error!, /interrupted/);
  assert.deepEqual(kept.inserted, ["Keep me"]);
  kept.h.unmount();
  const late = setup();
  await late.begin();
  late.result("Heard", true);
  late.emit("error", { error: "no-speech", message: "" });
  late.emit("end", null);
  assert.equal(late.h.value().phase, "idle");
  assert.deepEqual(late.inserted, ["Heard"]);
  assert.equal(
    late.h.value().error,
    undefined,
    "no-speech after words is not an error",
  );
  late.h.unmount();
});

test("a chosen dictation language the recognizer supports wins over the phone's", async () => {
  const chosen = setup({ chosen: "en-US" });
  await chosen.begin();
  assert.equal(chosen.native.options?.lang, "en-US");
  chosen.h.unmount();
  // A choice this iPhone cannot recognize falls back to the phone's language.
  const unsupported = setup({ chosen: "xx-YY" });
  await unsupported.begin();
  assert.equal(unsupported.native.options?.lang, "en-GB");
  unsupported.h.unmount();
});

test("crew speech uses native utterance boundaries and delivers the final words once", async () => {
  const x = setup({ continuous: false });
  await x.begin();
  assert.equal(x.native.options?.continuous, false);
  x.native.emit("result", {
    isFinal: true,
    results: [{ transcript: "Review the launch" }],
  });
  x.native.emit("end");
  x.h.flush();
  x.native.emit("end");
  assert.deepEqual(x.inserted, ["Review the launch"]);
  x.h.unmount();
});

test("crew speech never dispatches an interrupted or still-interim utterance", async () => {
  for (const failed of [false, true]) {
    const x = setup({ continuous: false });
    await x.begin();
    x.native.emit("result", {
      isFinal: failed,
      results: [{ transcript: "unfinished words" }],
    });
    if (failed) x.native.emit("error", { error: "interrupted" });
    x.native.emit("end");
    x.h.flush();
    assert.deepEqual(x.inserted, []);
    assert.ok(x.h.value().error);
    x.h.unmount();
  }
});
