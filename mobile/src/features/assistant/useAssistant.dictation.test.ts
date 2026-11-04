import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { historySchema } from "./assistantTypes";
import type { useAssistant } from "./useAssistant";
import type { useComposerDictation } from "./useComposerDictation";
import type {
  ComposerDictationButton,
  ComposerDictationReview,
} from "./ComposerDictation";
import type { AssistantDictationClient } from "./voice/AssistantDictationClient";

type Element = { type: string; props: Record<string, any> };
type DictationClientOptions = ConstructorParameters<
  typeof AssistantDictationClient
>[0];
const CHAT = `nyxa-${"a".repeat(32)}`;
const agent = {
  id: "nyxbot",
  kind: "nyxbot",
  status: "idle",
  can_use: true,
  model: "nyxagent/chat",
};

function elements(element: Element): Element[] {
  return [
    element,
    ...[element.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

async function setup(saved: boolean, waitForInitialHistory = false) {
  let appState = "active";
  let refreshing = false;
  let loadingInitial = waitForInitialHistory;
  let focused = true;
  const listeners = new Set<(state: string) => void>();
  const permission = deferred<{ granted: boolean }>();
  const refresh = deferred<ReturnType<typeof historySchema.parse>>();
  const events: string[] = [];
  const inserted: string[] = [];
  const page = historySchema.parse({
    conversation: {
      id: CHAT,
      title: "Saved QA chat",
      model: "nyxagent/chat",
      created_at: "2026-11-02T00:00:00Z",
      last_message_at: "2026-11-02T00:00:00Z",
      message_count: 0,
      active_turn: null,
    },
    messages: [],
    acknowledgements: [],
    approvals: [],
    waiting: [],
    before_seq: null,
  });
  const h = hookHarness<{
    assistant: ReturnType<typeof useAssistant>;
    dictation: ReturnType<typeof useComposerDictation>;
    button: Element;
    review: Element | null;
  }>({
    "react/jsx-runtime": Object.fromEntries(
      ["jsx", "jsxs"].map((key) => [
        key,
        (type: string, props: any) => ({ type, props }),
      ]),
    ),
    "react-native": {
      AppState: {
        get currentState() {
          return appState;
        },
        addEventListener: (_: string, callback: (state: string) => void) => {
          listeners.add(callback);
          return { remove: () => listeners.delete(callback) };
        },
      },
      Linking: {
        openSettings: async () => {
          events.push("settings");
        },
      },
      ActivityIndicator: "ActivityIndicator",
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
    "@tanstack/react-query": {
      useQuery: () => ({ data: { agents: [agent] }, refetch: async () => {} }),
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
    "../../lib/accessibility/workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({ sheetOpen: false }),
    },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "../../lib/api/assistantApi": {
      assistantApi: {
        history: async () => {
          if (refreshing || loadingInitial) return refresh.promise;
          return page;
        },
        list: async () => ({ conversations: [], next_cursor: null }),
      },
    },
    "expo-camera": {
      Camera: {
        requestMicrophonePermissionsAsync: () => {
          events.push("permission");
          return permission.promise;
        },
      },
    },
    "expo-crypto": { randomUUID: () => "id" },
    "../../lib/api/assistantVoiceApi": {
      nativeVoiceEnvironment: () => ({
        epoch: 1,
        origin: "https://nyx.example",
        api: {
          dictationOptions: async () => {
            events.push("options");
            return {
              options: [
                {
                  available: true,
                  default_model: true,
                  service_id: "service",
                  model: "transcribe",
                },
              ],
            };
          },
        },
        createSocket: () => {},
      }),
    },
    "./voice/AssistantDictationClient": {
      AssistantDictationClient: class {
        constructor(private options: DictationClientOptions) {}
        async start() {
          events.push("start");
          this.options.onPhase("listening");
        }
        stop() {
          this.options.onPhase("review");
        }
        cancel() {
          events.push("cancel");
        }
      },
    },
    "./voice/VoiceMediaBridge": {
      NativeVoiceMedia: class {
        close() {
          events.push("media-close");
        }
      },
      VoiceMediaBridge: "VoiceMediaBridge",
    },
    "lucide-react-native": { Mic: "Mic", Square: "Square" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Label: "Label",
    },
    "../../components/workspace/ScrollEdgeFades": {
      OverflowScrollView: "OverflowScrollView",
    },
  });
  const assistantHook = h.load(
    fileURLToPath(new URL("./useAssistant.ts", import.meta.url)),
  ).useAssistant as typeof useAssistant;
  const dictationHook = h.load(
    fileURLToPath(new URL("./useComposerDictation.ts", import.meta.url)),
  ).useComposerDictation as typeof useComposerDictation;
  const components = h.load(
    fileURLToPath(new URL("./ComposerDictation.tsx", import.meta.url)),
  ) as {
    ComposerDictationButton: typeof ComposerDictationButton;
    ComposerDictationReview: typeof ComposerDictationReview;
  };
  const render = () => {
    const assistant = assistantHook(saved ? CHAT : undefined);
    // The screen's existing eligibility gate: foreground history loading blocks composition.
    const enabled =
      !assistant.loading &&
      !assistant.sending &&
      !assistant.history?.conversation.active_turn;
    const dictation = dictationHook({
      scope: `${assistant.agentId}:${assistant.conversationId ?? "new"}:0`,
      enabled,
      getThread: async () => {
        events.push("thread");
        return assistant.conversationId ?? CHAT;
      },
      onInsert: (text) => inserted.push(text),
    });
    return {
      assistant,
      dictation,
      button: components.ComposerDictationButton({
        dictation,
        disabled: !enabled,
      }) as unknown as Element,
      review: components.ComposerDictationReview({
        dictation,
      }) as unknown as Element | null,
    };
  };
  h.mount(render);
  await h.settle();
  return {
    h,
    events,
    inserted,
    permission,
    refresh: () => {
      refreshing = true;
    },
    app: (state: string) => {
      appState = state;
      for (const listener of [...listeners]) listener(state);
      h.flush();
    },
    blur: () => {
      focused = false;
      h.render();
    },
    reopen: () => {
      loadingInitial = true;
      h.value().assistant.openConversation(CHAT);
      h.flush();
    },
    release: async () => {
      refresh.resolve(page);
      await h.settle();
    },
  };
}

test("saved and new chat microphone taps retain pending permission and start after iOS foreground restoration", async () => {
  for (const saved of [true, false]) {
    const x = await setup(saved);
    assert.equal(x.h.value().button.props.disabled, false);
    x.h.value().button.props.onPress();
    x.h.flush();
    assert.equal(x.h.value().dictation.phase, "permission");
    assert.ok(
      elements(x.h.value().review!).some(
        (node) => node.props.children === "Waiting for microphone permission…",
      ),
    );
    x.app("inactive");
    x.permission.resolve({ granted: true });
    await x.h.settle();
    assert.deepEqual(x.events, ["permission"]);
    x.refresh();
    x.app("active");
    await x.h.settle();
    assert.equal(
      x.h.value().assistant.loading,
      false,
      saved ? "saved history refresh must not cancel dictation" : "new chat",
    );
    assert.equal(x.h.value().dictation.phase, "listening");
    assert.equal(x.h.value().button.props.accessibilityLabel, "Stop dictation");
    assert.deepEqual(x.events, ["permission", "thread", "options", "start"]);
    assert.deepEqual(x.inserted, []);
    await x.release();
    assert.equal(x.h.value().dictation.phase, "listening");
    x.h.unmount();
  }
});

test("saved chat permission denial remains visible while its foreground history refresh is pending", async () => {
  const x = await setup(true);
  x.h.value().button.props.onPress();
  x.app("inactive");
  x.permission.resolve({ granted: false });
  await x.h.settle();
  x.refresh();
  x.app("active");
  await x.h.settle();
  assert.equal(x.h.value().dictation.permissionDenied, true);
  assert.match(x.h.value().dictation.error!, /Allow microphone access/);
  const settings = elements(x.h.value().review!).find(
    (node) => node.props.label === "Open Settings",
  )!;
  settings.props.onPress();
  assert.deepEqual(x.events, ["permission", "settings"]);
  await x.release();
  assert.equal(x.h.value().dictation.permissionDenied, true);
  x.h.unmount();
});

test("already granted microphone permission starts saved and new chat dictation on the first tap", async () => {
  for (const saved of [true, false]) {
    const x = await setup(saved);
    x.permission.resolve({ granted: true });
    x.h.value().button.props.onPress();
    await x.h.settle();
    assert.equal(x.h.value().dictation.phase, "listening");
    assert.deepEqual(x.events, ["permission", "thread", "options", "start"]);
    assert.deepEqual(x.inserted, []);
    x.h.unmount();
  }
});

test("initial saved-history loading and explicit thread reopen still block dictation", async () => {
  for (const initial of [true, false]) {
    const x = await setup(true, initial);
    if (!initial) x.reopen();
    assert.equal(x.h.value().assistant.loading, true);
    assert.equal(x.h.value().button.props.disabled, true);
    await x.h.value().dictation.start();
    assert.deepEqual(x.events, []);
    await x.release();
    assert.equal(x.h.value().assistant.loading, false);
    x.h.unmount();
  }
});

test("background and navigation blur still cancel a saved chat's pending permission", async () => {
  for (const action of ["background", "blur"]) {
    const x = await setup(true);
    x.h.value().button.props.onPress();
    if (action === "background") x.app("background");
    else x.blur();
    x.permission.resolve({ granted: true });
    await x.h.settle();
    assert.equal(x.h.value().dictation.phase, "idle");
    assert.deepEqual(x.events, ["permission"]);
    x.h.unmount();
  }
});
