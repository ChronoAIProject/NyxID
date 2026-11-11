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

type Element = { type: unknown; props: Record<string, any> };
type Listener = (event?: any) => void;
const CHAT = `nyxa-${"a".repeat(32)}`;
const agent = {
  id: "nyxbot",
  kind: "nyxbot",
  status: "idle",
  can_use: true,
  model: "nyxagent/chat",
};

function elements(element: Element | null): Element[] {
  if (!element) return [];
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

// Expand local components in their own snapshot harness, keeping their hooks
// separate from the assistant/dictation lifecycle under test.
function expand(node: any): any {
  if (Array.isArray(node)) return node.map(expand);
  if (!node || typeof node !== "object" || !("props" in node)) return node;
  if (typeof node.type === "function") return expand(node.type(node.props));
  const children =
    typeof node.props.children === "function"
      ? node.props.children({ pressed: false })
      : node.props.children;
  return { ...node, props: { ...node.props, children: expand(children) } };
}

async function setup(saved: boolean, waitForInitialHistory = false) {
  let appState = "active";
  let refreshing = false;
  let loadingInitial = waitForInitialHistory;
  let focused = true;
  let epoch = 1;
  let generation = 0;
  const listeners = new Set<(state: string) => void>();
  const nativeListeners = new Map<string, Set<Listener>>();
  const permission = deferred<{ granted: boolean }>();
  const pendingHistory: ReturnType<
    typeof deferred<ReturnType<typeof historySchema.parse>>
  >[] = [];
  const events: string[] = [];
  const inserted: string[] = [];
  const emit = (name: string, event?: unknown) => {
    for (const listener of [...(nativeListeners.get(name) ?? [])])
      listener(event);
  };
  const native = {
    requestPermissionsAsync: () => {
      events.push("permission");
      return permission.promise;
    },
    getSupportedLocales: async () => ({ locales: ["en-US"] }),
    supportsOnDeviceRecognition: () => true,
    addListener: (name: string, listener: Listener) => {
      const set = nativeListeners.get(name) ?? new Set();
      nativeListeners.set(name, set);
      set.add(listener);
      return { remove: () => set.delete(listener) };
    },
    start: () => events.push("start"),
    stop: () => events.push("stop"),
    abort: () => {
      events.push("abort");
      emit("end");
    },
  };
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
  const mocks = {
    "react/jsx-runtime": Object.fromEntries(
      ["jsx", "jsxs"].map((key) => [
        key,
        (type: string, props: any) => ({ type, props }),
      ]),
    ),
    "react-native": {
      Platform: { OS: "ios" },
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
      Keyboard: { dismiss() {} },
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      Text: "Text",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
    "@tanstack/react-query": {
      useQuery: () => ({ data: { agents: [agent] }, refetch: async () => {} }),
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/accessibility/workspaceAccessibility": {
      useWorkspaceAccessibility: () => ({ sheetOpen: false }),
    },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "./useAssistantLiveRefresh": { useAssistantLiveRefresh: () => {} },
    "../../lib/api/assistantApi": {
      assistantApi: {
        history: async () => {
          if (refreshing || loadingInitial) {
            const request = deferred<ReturnType<typeof historySchema.parse>>();
            pendingHistory.push(request);
            return request.promise;
          }
          return page;
        },
        list: async () => ({ conversations: [], next_cursor: null }),
      },
    },
    "expo-speech-recognition": { ExpoSpeechRecognitionModule: native },
    "./voice/dictationLanguage": {
      savedDictationLanguage: async () => "en-US",
    },
    "lucide-react-native": {
      Check: "Check",
      Mic: "Mic",
      Square: "Square",
      X: "X",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Label: "Label",
    },
    "../../components/workspace/ScrollEdgeFades": {
      OverflowScrollView: "OverflowScrollView",
    },
  };
  const h = hookHarness<{
    assistant: ReturnType<typeof useAssistant>;
    dictation: ReturnType<typeof useComposerDictation>;
    button: Element;
  }>(mocks);
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
      scope: `${assistant.agentId}:${assistant.conversationId ?? "new"}:${generation}`,
      enabled,
      onInsert: (text) => inserted.push(text),
    });
    return {
      assistant,
      dictation,
      button: components.ComposerDictationButton({
        dictation,
        disabled: !enabled,
      }) as unknown as Element,
    };
  };
  h.mount(render);
  await h.settle();
  return {
    h,
    events,
    inserted,
    permission,
    feedback: () => {
      const ui = hookHarness<Element | null>(mocks);
      const review = ui.load(
        fileURLToPath(new URL("./ComposerDictation.tsx", import.meta.url)),
      ).ComposerDictationReview as typeof ComposerDictationReview;
      ui.mount(() => expand(review({ dictation: h.value().dictation })));
      const nodes = elements(ui.value());
      ui.unmount();
      return nodes;
    },
    emit: (name: string, event?: unknown) => {
      emit(name, event);
      h.flush();
    },
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
    account: () => {
      epoch++;
      h.render();
    },
    scope: () => {
      generation++;
      h.render();
    },
    reopen: () => {
      loadingInitial = true;
      h.value().assistant.openConversation(CHAT);
      h.flush();
    },
    releaseOldest: async () => {
      assert.ok(pendingHistory.length);
      pendingHistory.shift()!.resolve(page);
      await h.settle();
    },
    release: async (activeTurn = false) => {
      const response = activeTurn
        ? historySchema.parse({
            ...page,
            conversation: {
              ...page.conversation,
              active_turn: {
                turn_id: "active-turn",
                started_at: "2026-11-02T00:00:00Z",
              },
            },
          })
        : page;
      for (const request of pendingHistory.splice(0)) request.resolve(response);
      await h.settle();
    },
    failHistory: async () => {
      for (const request of pendingHistory.splice(0))
        request.reject(new Error("History refresh failed."));
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
      x
        .feedback()
        .some((node) => node.props.children === "Waiting for microphone…"),
    );
    assert.ok(
      x
        .feedback()
        .some((node) => node.props.accessibilityLabel === "Discard dictation"),
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
    assert.equal(x.h.value().dictation.phase, "connecting");
    assert.deepEqual(x.events, ["permission", "start"]);
    x.emit("start");
    assert.equal(x.h.value().dictation.phase, "listening");
    assert.equal(x.h.value().button.props.accessibilityLabel, "Stop dictation");
    assert.deepEqual(x.inserted, []);
    await x.release();
    assert.equal(x.h.value().dictation.phase, "listening");
    x.h.unmount();
  }
});

test("saved and new chat permission denial remains actionable during a foreground history refresh", async () => {
  for (const saved of [true, false]) {
    const x = await setup(saved);
    x.h.value().button.props.onPress();
    x.app("inactive");
    x.permission.resolve({ granted: false });
    await x.h.settle();
    x.refresh();
    x.app("active");
    await x.h.settle();
    assert.equal(x.h.value().dictation.permissionDenied, true);
    assert.match(
      x.h.value().dictation.error!,
      /Allow microphone and speech recognition access/,
    );
    const settings = x
      .feedback()
      .find((node) => node.props.label === "Open Settings")!;
    settings.props.onPress();
    assert.deepEqual(x.events, ["permission", "settings"]);
    await x.release();
    assert.equal(x.h.value().dictation.permissionDenied, true);
    assert.deepEqual(x.inserted, []);
    x.h.unmount();
  }
});

test("already granted microphone permission starts saved and new chat dictation on the first tap", async () => {
  for (const saved of [true, false]) {
    const x = await setup(saved);
    x.permission.resolve({ granted: true });
    x.h.value().button.props.onPress();
    await x.h.settle();
    assert.deepEqual(x.events, ["permission", "start"]);
    x.emit("start");
    assert.equal(x.h.value().dictation.phase, "listening");
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

test("foreground restoration keeps initial history and an explicit saved-thread reopen blocked until history settles", async () => {
  for (const initial of [true, false]) {
    const x = await setup(true, initial);
    if (!initial) x.reopen();
    assert.equal(x.h.value().assistant.loading, true);
    x.app("inactive");
    x.app("active");
    await x.h.settle();
    assert.equal(x.h.value().assistant.loading, true);
    assert.equal(x.h.value().button.props.disabled, true);
    await x.h.value().dictation.start();
    assert.deepEqual(x.events, []);
    // The original reopen response is superseded by foreground restoration.
    // It cannot release the latest foreground request's blocking state.
    await x.releaseOldest();
    assert.equal(x.h.value().assistant.loading, true);
    assert.equal(x.h.value().button.props.disabled, true);
    await x.release();
    assert.equal(x.h.value().assistant.loading, false);
    assert.equal(x.h.value().button.props.disabled, false);
    x.h.unmount();
  }
});

test("background, navigation, account and scope changes still cancel a saved chat's pending permission", async () => {
  for (const action of ["background", "blur", "account", "scope"]) {
    const x = await setup(true);
    x.h.value().button.props.onPress();
    if (action === "background") x.app("background");
    if (action === "blur") x.blur();
    if (action === "account") x.account();
    if (action === "scope") x.scope();
    x.permission.resolve({ granted: true });
    await x.h.settle();
    assert.equal(x.h.value().dictation.phase, "idle");
    assert.deepEqual(x.events, ["permission"]);
    x.h.unmount();
  }
});

test("a failed explicit reopen refresh releases loading and reports the failure", async () => {
  const x = await setup(true);
  x.reopen();
  x.app("inactive");
  x.app("active");
  await x.failHistory();
  assert.equal(x.h.value().assistant.loading, false);
  assert.equal(
    x.h.value().assistant.error,
    "Could not reach the assistant. Try again.",
  );
  assert.equal(x.h.value().button.props.disabled, false);
  assert.deepEqual(x.events, []);
  x.h.unmount();
});

test("a foreground history response with an active turn cancels pending permission", async () => {
  const x = await setup(true);
  x.h.value().button.props.onPress();
  x.app("inactive");
  x.refresh();
  x.app("active");
  await x.release(true);
  assert.equal(x.h.value().dictation.phase, "idle");
  assert.equal(x.h.value().button.props.disabled, true);
  x.permission.resolve({ granted: true });
  await x.h.settle();
  assert.deepEqual(x.events, ["permission"]);
  assert.deepEqual(x.inserted, []);
  x.h.unmount();
});
