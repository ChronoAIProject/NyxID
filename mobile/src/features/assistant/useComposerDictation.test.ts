import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { useComposerDictation } from "./useComposerDictation";
import type { AssistantDictationClient } from "./voice/AssistantDictationClient";

type Options = ConstructorParameters<typeof AssistantDictationClient>[0];
const option = {
  service_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  model: "gpt-live-transcribe",
  available: true,
  default_model: true,
};
function setup() {
  let state = "active",
    focused = true,
    sheetOpen = false,
    epoch = 1;
  const permissions = deferred<{ granted: boolean }>();
  const appListeners = new Set<(state: string) => void>();
  const events: string[] = [],
    inserted: string[] = [];
  let options: Options;
  let resolveForeground: (() => void) | undefined;
  const clients: Client[] = [];
  const props = {
    scope: "agent:chat:0",
    enabled: true,
    getThread: async () => {
      events.push("thread");
      return "nyxa-" + "a".repeat(32);
    },
    onInsert: (text: string) => {
      inserted.push(text);
    },
  };
  class Media {
    close() {
      events.push("media-close");
    }
  }
  class Client {
    constructor(value: Options) {
      options = value;
      clients.push(this);
    }
    async start(thread: string, preferences: any) {
      events.push("start");
      assert.equal(thread, "nyxa-" + "a".repeat(32));
      assert.deepEqual(JSON.parse(JSON.stringify(preferences)), {
        service_id: option.service_id,
        model: option.model,
        connection_id: null,
        key_source: "platform",
        voice: null,
        language: null,
        input_mode: "push_to_talk",
        notify_on_completion: false,
      });
      options.onPhase("listening");
    }
    stop() {
      events.push("stop");
      options.onPhase("stopping");
    }
    cancel() {
      events.push("cancel");
    }
  }
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
    "expo-camera": {
      Camera: {
        requestMicrophonePermissionsAsync: () => {
          events.push("permission");
          return permissions.promise;
        },
      },
    },
    "expo-crypto": { randomUUID: () => "id" },
    "@react-navigation/native": { useIsFocused: () => focused },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/accessibility/workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({ sheetOpen }),
    },
    "../../lib/api/assistantVoiceApi": {
      nativeVoiceEnvironment: () => ({
        epoch,
        origin: "https://nyx.example",
        api: {
          dictationOptions: async () => {
            events.push("options");
            return {
              options: [
                { ...option, default_model: false, model: "other" },
                option,
              ],
            };
          },
        },
        createSocket: () => {},
      }),
    },
    "./voice/AssistantDictationClient": { AssistantDictationClient: Client },
    "./voice/VoiceMediaBridge": { NativeVoiceMedia: Media },
    "./voice/voiceSelection": {
      chooseVoiceOption: (items: any[]) =>
        items.find((v) => v.available && v.default_model) ??
        items.find((v) => v.available),
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
  const result = (text: string) => {
    options.onText({ final: text, interim: "" });
    h.flush();
  };
  const finish = () => {
    options.onPhase("review");
    h.flush();
  };
  return {
    h,
    props,
    events,
    inserted,
    permissions,
    clients,
    result,
    finish,
    append: module.appendDictation as (draft: string, text: string) => string,
    begin: async () => {
      const pending = h.value().start();
      permissions.resolve({ granted: true });
      await pending;
      await h.settle();
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
    error: (message: string) => options.onError(message),
  };
}

test("server dictation requests permission, shared draft and default options in order; review inserts once", async () => {
  const x = setup();
  await x.begin();
  assert.deepEqual(x.events, ["permission", "thread", "options", "start"]);
  x.result("Hello world");
  assert.deepEqual(x.inserted, []);
  x.h.value().stop();
  x.finish();
  const insert = x.h.value().insert;
  insert();
  insert();
  assert.deepEqual(x.inserted, ["Hello world"]);
  assert.equal(x.append("Existing\n", "Hello"), "Existing\nHello");
  assert.equal(x.append("Existing", "Hello"), "Existing Hello");
  x.h.unmount();
});
test("discard releases audio and ignores late server results without inserting", async () => {
  const x = setup();
  await x.begin();
  x.result("Private");
  x.h.value().discard();
  x.h.flush();
  x.result("Late");
  x.finish();
  assert.equal(x.h.value().phase, "idle");
  assert.equal(x.h.value().text, "");
  assert.deepEqual(x.inserted, []);
  assert.ok(x.events.includes("cancel"));
  x.h.unmount();
});
test("account/chat/access/focus/sheet changes fence capture and stale review callbacks", async () => {
  for (const change of ["account", "chat", "disabled", "focus", "sheet"]) {
    const x = setup();
    await x.begin();
    x.result("Old");
    x.change(change);
    x.result("Late");
    x.finish();
    assert.equal(x.h.value().phase, "idle", change);
    assert.deepEqual(x.inserted, []);
    x.h.unmount();
    const y = setup();
    await y.begin();
    y.result("Old review");
    y.finish();
    const insert = y.h.value().insert;
    y.change(change);
    insert();
    assert.deepEqual(y.inserted, [], change);
    y.h.unmount();
  }
});
test("late permission cannot create a draft or connect after cancellation or unmount", async () => {
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
    x.permissions.resolve({ granted: true });
    await pending;
    assert.deepEqual(x.events, ["permission"], change);
    if (change !== "unmount") x.h.unmount();
  }
});
test("permission denial links to Settings and never creates a thread", async () => {
  const x = setup();
  const pending = x.h.value().start();
  x.permissions.resolve({ granted: false });
  await pending;
  await x.h.settle();
  assert.match(x.h.value().error!, /Settings/);
  assert.equal(x.h.value().permissionDenied, true);
  assert.equal(x.h.value().active, false);
  x.h.value().openSettings();
  assert.deepEqual(x.events, ["permission", "settings"]);
  x.h.unmount();
});
test("permission inactive waits for foreground; background cancels capture", async () => {
  const x = setup();
  const pending = x.h.value().start();
  x.app("inactive");
  x.permissions.resolve({ granted: true });
  await x.h.settle();
  assert.deepEqual(x.events, ["permission"]);
  x.app("active");
  await pending;
  await x.h.settle();
  x.result("Private");
  x.app("background");
  assert.equal(x.h.value().text, "");
  assert.equal(x.h.value().active, false);
  x.h.unmount();
});
test("hold release before permission completes cancels without creating a paid session", async () => {
  const x = setup();
  const pending = x.h.value().start();
  x.h.value().stop();
  x.permissions.resolve({ granted: true });
  await pending;
  assert.deepEqual(x.events, ["permission"]);
  x.h.unmount();
});
test("server failure preserves completed review text and never auto-sends", async () => {
  const x = setup();
  await x.begin();
  x.result("Review me");
  x.error("Connection lost");
  x.finish();
  assert.equal(x.h.value().phase, "review");
  assert.equal(x.h.value().text, "Review me");
  assert.deepEqual(x.inserted, []);
  x.h.unmount();
});
