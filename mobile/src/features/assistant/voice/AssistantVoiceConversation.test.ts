import {
  composerInputMinHeight,
  composerVoiceMinHeight,
} from "../composerGeometry";
import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "../assistantHookTestHarness";
import type { VoiceMediaEvent } from "./AssistantVoiceClient";
import type { VoiceStartRecovery } from "./voiceStartError";
import type {
  VoiceOption,
  VoicePreferences,
  VoiceSnapshot,
  VoiceSession,
} from "./voiceContracts";

type Element = { type: unknown; key?: unknown; props: Record<string, any> };
const serviceId = "11111111-1111-4111-8111-111111111111";
const sessionId = "22222222-2222-4222-8222-222222222222";
const thread = `nyxa-${"a".repeat(32)}`;
const saved: VoicePreferences = {
  service_id: serviceId,
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  voice: "cedar",
  input_mode: "automatic",
  language: "zh-CN",
  notify_on_completion: true,
};
const source: VoiceOption = {
  service_id: serviceId,
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  model_label: "GPT Live",
  default_model: true,
  voice: {
    protocol: "openai_live",
    models: [{ id: "gpt-live-1", label: "GPT Live", default: true }],
    voices: [
      { id: "marin", label: "Marin" },
      { id: "cedar", label: "Cedar" },
    ],
    usage_source: "provider_reported",
    billing_metrics: ["voice_seconds"],
  },
  available: true,
  unavailable_reason: null,
  billing_owner: "person",
  pricing: null,
};
const snapshot: VoiceSnapshot = {
  type: "snapshot",
  session: {
    created_at: "2026-10-16T04:00:00Z",
    closed_at: null,
    id: sessionId,
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
  },
  captions: [],
  tasks: [],
};

function expand(element: Element): Element {
  if (typeof element.type === "function")
    return expand((element.type as (props: unknown) => Element)(element.props));
  return {
    ...element,
    props: {
      ...element.props,
      children: [element.props.children]
        .flat(Infinity)
        .map((child) =>
          child && typeof child === "object" && "props" in child
            ? expand(child)
            : child,
        ),
    },
  };
}
function elements(root: Element): Element[] {
  return [
    root,
    ...root.props.children
      .flat(Infinity)
      .flatMap((child: unknown) =>
        child && typeof child === "object" && "props" in child
          ? elements(child as Element)
          : [],
      ),
  ];
}
function setup(
  input: {
    viewport?: { width: number; height: number; fontScale: number };
    active?: boolean;
    source?: VoiceOption;
    options?: VoiceOption[];
    saved?: VoicePreferences | null;
    settingsPending?: boolean;
    optionsPending?: boolean;
    snapshot?: VoiceSnapshot;
    error?: string;
    recovery?: VoiceStartRecovery;
    reducedMotion?: boolean;
    conflict?: VoiceSession;
    conflictThread?: string;
    conflictUnknown?: boolean;
    checkingConflict?: boolean;
    inputEnabled?: boolean;
    preferencesOpen?: boolean;
  } = {},
) {
  const events: unknown[][] = [];
  const layoutAnimations: unknown[] = [];
  let focused = true;
  let online = true;
  const keyboard = new Map<string, (event: any) => void>();
  const appListeners = new Set<(state: string) => void>();
  const intervals = new Map<object, () => void>();
  let browserFails = false;
  let saveFails = false;
  let endResult: Promise<boolean> = Promise.resolve(true);
  let checkResult: Promise<boolean> = Promise.resolve(false);
  let endConflictResult: Promise<boolean> = Promise.resolve(false);
  let onStart: () => unknown = () => {};
  const savedPreferences: VoicePreferences[] = [];
  const snapshots: (VoiceSnapshot | undefined)[] = [];
  const listeners = new Set<(event: VoiceMediaEvent) => void>();
  const environment = {
    origin: "https://nyx.example",
    epoch: 7,
    api: {
      savePreferences: async (preferences: VoicePreferences) => {
        events.push(["save", preferences.input_mode]);
        if (saveFails) throw new Error("private save error");
        savedPreferences.push(preferences);
      },
    },
  };
  const levels = {
    input: { value: 0 },
    output: { value: 0 },
    speaking: false,
    media: undefined as unknown,
    subscribeLevel: () => () => {},
  };
  const media = {
    id: 1,
    subscribe: (listener: (event: VoiceMediaEvent) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  const voice = {
    media: media as typeof media | null,
    mediaAttempt: 1,
    starting: false,
    connected: input.active ?? false,
    connectedAt: input.active ? Date.now() : undefined,
    disconnectedAt: undefined as number | undefined,
    inputEnabled: input.inputEnabled ?? input.active ?? false,
    holding: false,
    speakerMuted: false,
    snapshot: input.snapshot ?? (input.active ? snapshot : null),
    error: input.error,
    recovery: input.recovery,
    conflict: input.conflict ?? (null as VoiceSession | null),
    conflictThread: input.conflictThread,
    conflictUnknown: input.conflictUnknown ?? false,
    endingConflict: false,
    checkingConflict: input.checkingConflict ?? false,
    confirmedEnded: false,
    start: async (preferences: VoicePreferences, protocol: string) => {
      events.push(["start", preferences, protocol]);
      return onStart();
    },
    end: () => {
      events.push(["end"]);
      voice.inputEnabled = false;
      return endResult;
    },
    endConflict: () => {
      events.push(["endConflict"]);
      return endConflictResult;
    },
    checkConflict: (manual = true) => {
      events.push(manual ? ["checkConflict"] : ["checkConflict", false]);
      return checkResult;
    },
    hold: (value: boolean) => {
      events.push(["hold", value]);
      voice.holding = value;
      voice.inputEnabled = value;
    },
    mute: async (value: boolean) => {
      events.push(["mute", value]);
      voice.inputEnabled = !value;
    },
    muteSpeaker: (value: boolean) => {
      events.push(["speaker", value]);
      voice.speakerMuted = value;
    },
    setInputMode: (mode: string) => {
      events.push(["mode", mode]);
      return true;
    },
    resumeAudio: async () => {
      events.push(["resume"]);
    },
    stopTask: async (id: string) => {
      events.push(["stopTask", id]);
    },
  };
  const query = {
    options: {
      data: { options: input.options ?? [input.source ?? source] },
      isSuccess: !input.optionsPending,
      isPending: !!input.optionsPending,
      error: null as Error | null,
      refetch: async () => {
        events.push(["optionsRefetch"]);
      },
    },
    settings: {
      data: input.settingsPending
        ? undefined
        : { voice: input.saved === undefined ? saved : input.saved },
      isSuccess: !input.settingsPending,
      isPending: !!input.settingsPending,
      error: null as Error | null,
      refetch: async () => {
        events.push(["settingsRefetch"]);
      },
    },
  };
  const props = {
    thread,
    agentName: "我的助手",
    onClose: () => {
      events.push(["close"]);
    },
    onUpdate: () => {},
    onReview: (id?: string) => {
      events.push(["review", id]);
    },
    minimized: false,
    onExpand: () => {
      events.push(["expand"]);
    },
    preferencesOpen: input.preferencesOpen ?? false,
    onSnapshot: (value: VoiceSnapshot | undefined) => snapshots.push(value),
    onPreferencesClose: () => {
      props.preferencesOpen = false;
    },
  };
  const harness = hookHarness<Element>(
    {
      "../../../components/workspace/useNativeSheetDismiss": {
        useNativeSheetDismiss: (onClose: () => void) => ({
          modalProps: { visible: true, animationType: "slide" },
          close: () => onClose(),
        }),
      },
      "../../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => input.reducedMotion ?? false,
      },
      "react/jsx-runtime": {
        Fragment: "Fragment",
        jsx: (type: unknown, props: unknown, key?: unknown) => ({
          type,
          props,
          keyboard,
          key,
        }),
        jsxs: (type: unknown, props: unknown, key?: unknown) => ({
          type,
          props,
          keyboard,
          key,
        }),
      },
      "react-native": {
        ...Object.fromEntries(
          [
            "ActivityIndicator",
            "Modal",
            "Pressable",
            "ScrollView",
            "Text",
            "TextInput",
            "View",
          ].map((name) => [name, name]),
        ),
        Keyboard: {
          addListener: (name: string, callback: (event: any) => void) => {
            keyboard.set(name, callback);
            return { remove: () => keyboard.delete(name) };
          },
        },
        AppState: {
          currentState: "active",
          addEventListener: (
            _name: string,
            callback: (state: string) => void,
          ) => {
            appListeners.add(callback);
            return { remove: () => appListeners.delete(callback) };
          },
        },
        LayoutAnimation: {
          configureNext: (preset: unknown) => layoutAnimations.push(preset),
          Presets: { easeInEaseOut: "easeInEaseOut" },
        },
        StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 1 },
        useWindowDimensions: () =>
          input.viewport ?? { width: 390, height: 844, fontScale: 1 },
        Animated: {
          View: "Animated.View",
          Value: class {
            setValue() {}
            interpolate() {
              return 0;
            }
          },
          spring: () => ({ start() {}, stop() {} }),
          timing: () => ({ start() {}, stop() {} }),
        },
      },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ top: 47, bottom: 34, left: 0, right: 0 }),
      },
      "@react-navigation/native": { useIsFocused: () => focused },
      "expo-web-browser": {
        openBrowserAsync: async (url: string) => {
          events.push(["billing", url]);
          if (browserFails) throw new Error("private device error");
        },
      },
      "@tanstack/react-query": {
        useQuery: ({ queryKey }: { queryKey: unknown[] }) =>
          queryKey.includes("voice-options") ? query.options : query.settings,
      },
      "expo-linear-gradient": { LinearGradient: "LinearGradient" },
      "lucide-react-native": Object.fromEntries(
        [
          "ChevronDown",
          "ChevronLeft",
          "ChevronUp",
          "MessageSquareText",
          "X",
          "Mic",
          "MicOff",
          "CircleHelp",
          "Phone",
          "Radio",
          "PhoneOff",
          "Volume2",
          "VolumeX",
        ].map((name) => [name, name]),
      ),
      "../ChatComposer": { ChatComposerFrame: "ComposerFrame" },
      "../../../components/workspace/ScrollEdgeFades": {
        ScrollEdgeFades: "ScrollEdgeFades",
        useScrollEdges: () => ({
          edges: { start: false, end: false },
          layout() {},
          content() {},
          scroll() {},
        }),
      },
      "../../../components/workspace/WorkspaceSheetSurface": {
        WorkspaceSheetSurface: "SheetSurface",
      },
      "../../../components/workspace/WorkspaceUI": {
        ActionButton: "ActionButton",
        Card: "Card",
        Label: "Text",
        Notice: "Notice",
      },
      "../../../components/settings/SettingsList": {
        SettingsRow: "SettingsRow",
      },
      "../../../components/workspace/WorkspaceSelectionField": {
        // The shared sheet owns its searchable draft and one-shot Apply action;
        // these tests exercise the voice screen's persistence boundary.
        WorkspaceSelectionSheet: "WorkspaceSelectionSheet",
      },
      "../../../lib/api/assistantVoiceApi": {
        nativeVoiceEnvironment: () => environment,
      },
      "../../../hooks/useNetworkStatus": {
        useNetworkStatus: () => ({ isConnected: online }),
      },
      "../../../theme/ThemeContext": {
        useTheme: () => ({
          colors: {
            primaryTone: { bg: "pink", border: "pink" },
            warningTone: { bg: "amber", text: "amber-text" },
            dangerTone: { bg: "red", text: "red-text" },
          },
        }),
      },
      "../../../theme/designTokens": { fonts: {} },
      "./useNativeVoice": { useNativeVoice: () => voice },
      "./useVoiceLevels": {
        useVoiceLevels: (media: unknown) => {
          levels.media = media;
          return levels;
        },
      },
      "../DictationWaveform": { DictationWaveform: "DictationWaveform" },
      "../composerGeometry": { composerInputMinHeight, composerVoiceMinHeight },
      "./VoiceComposerAura": { VoiceComposerAura: "VoiceAura" },
      "./VoiceMediaBridge": { VoiceMediaBridge: "VoiceMediaBridge" },
    },
    {
      setInterval: (callback: () => void) => {
        const token = {};
        intervals.set(token, callback);
        return token;
      },
      clearInterval: (token: object) => {
        intervals.delete(token);
      },
    },
  );
  harness.mock(
    "./VoiceComposerShell",
    harness.load(
      fileURLToPath(new URL("./VoiceComposerShell.tsx", import.meta.url)),
    ),
  );
  const component = harness.load(
    fileURLToPath(new URL("./AssistantVoiceConversation.tsx", import.meta.url)),
  ).AssistantVoiceConversation as (props: unknown) => Element;
  harness.mount(() => expand(component(props)));
  const all = () => elements(harness.flush());
  const find = (label: string) =>
    all().find(
      (row) =>
        row.props.accessibilityLabel === label ||
        ((row.type === "ActionButton" || row.type === "SettingsRow") &&
          row.props.label === label),
    );
  const one = (predicate: (row: Element) => boolean) => {
    const result = all().find(predicate);
    assert.ok(result, "Expected voice control");
    return result;
  };
  return {
    harness,
    props,
    keyboard,
    appState: (state: string) => {
      for (const listener of appListeners) listener(state);
      harness.flush();
    },
    runIntervals: () => {
      for (const callback of [...intervals.values()]) callback();
      harness.flush();
    },
    events,
    layoutAnimations,
    voice,
    levels,
    query,
    all,
    text: () =>
      all()
        .flatMap((row) => row.props.children.flat(Infinity))
        .filter((child) => typeof child === "string")
        .join(" "),
    failBrowser: () => {
      browserFails = true;
    },
    button: (label: string) => {
      const result = find(label);
      assert.ok(result, `Expected voice control: ${label}`);
      return result;
    },
    find,
    environment,
    savedPreferences,
    snapshots,
    voicePicker: () =>
      all().find((row) => row.type === "WorkspaceSelectionSheet"),
    notices: () =>
      all()
        .filter((row) => row.type === "Notice")
        .map((row) => String(row.props.message)),
    starts: () => events.filter(([event]) => event === "start"),
    openPreferences: () => {
      props.preferencesOpen = true;
      harness.render();
    },
    emit: (event: VoiceMediaEvent) => {
      for (const listener of listeners) listener(event);
      harness.flush();
    },
    failSave: () => {
      saveFails = true;
    },
    setEnd: (result: Promise<boolean>) => {
      endResult = result;
    },
    setCheck: (result: Promise<boolean>) => {
      checkResult = result;
    },
    setEndConflict: (result: Promise<boolean>) => {
      endConflictResult = result;
    },
    setStart: (callback: () => unknown) => {
      onStart = callback;
    },
    modal: () => one((row) => row.type === "Modal").props,
    aura: () => one((row) => row.type === "VoiceAura").props,
    media: () => one((row) => row.type === "VoiceMediaBridge"),
    setFocused: (value: boolean) => {
      focused = value;
      harness.render();
    },
    setOnline: (value: boolean) => {
      online = value;
      harness.render();
    },
  };
}

test("voice setup starts with the exact saved source, voice, input mode and notification preferences", async () => {
  const f = setup();
  assert.doesNotMatch(f.text(), /Tap to start/);
  assert.equal(f.events.length, 1);
  assert.equal(f.events[0]?.[0], "start");
  assert.deepEqual(JSON.parse(JSON.stringify(f.events[0]?.[1])), saved);
  assert.equal(f.events[0]?.[2], "openai_live");
  f.harness.unmount();
});

const creditFailure = {
  error: "Insufficient voice credits (billing_reservation · code 11300)",
  recovery: {
    kind: "credits" as const,
    message:
      "There aren’t enough available credits to reserve this voice call. Review your balance and applicable benefits in Billing, or choose another eligible voice source.",
  },
};

test("a rejected reservation shows a failed start and recovery while keeping diagnostics collapsed", () => {
  const f = setup(creditFailure);
  assert.match(f.text(), /Unable to start/);
  assert.doesNotMatch(f.text(), /Ready to talk|Connected to this conversation/);
  assert.equal(
    f.all().some((row) => row.props.message === creditFailure.recovery.message),
    true,
  );
  assert.equal(f.text().includes(creditFailure.error), false);
  assert.equal(f.button("Try again").props.disabled, false);
  f.button("Show error details").props.onPress();
  assert.equal(f.text().includes(creditFailure.error), true);
  assert.equal(
    f.button("Hide error details").props.accessibilityState.expanded,
    true,
  );
  f.voice.error = undefined;
  f.voice.recovery = undefined;
  f.voice.starting = true;
  f.harness.render();
  assert.match(f.text(), /Connecting/);
  assert.doesNotMatch(f.text(), /Unable to start|11300/);
  f.harness.unmount();
});

test("billing recovery opens the configured Web billing page and text recovery closes voice", async () => {
  const f = setup(creditFailure);
  await f.button("Review billing on Web").props.onPress();
  assert.deepEqual(f.events, [["billing", "https://nyx.example/billing"]]);
  // No call was created, so text recovery only closes voice.
  f.button("Continue with text").props.onPress();
  assert.deepEqual(f.events.slice(1), [["close"]]);
  f.harness.unmount();
});

test("billing browser errors retain the reservation error and provide fixed website guidance", async () => {
  const f = setup(creditFailure);
  f.failBrowser();
  await f.button("Review billing on Web").props.onPress();
  assert.equal(
    f
      .all()
      .some((row) => /Billing could not open/.test(row.props.message ?? "")),
    true,
  );
  assert.match(f.text(), /Unable to start/);
  assert.doesNotMatch(f.text(), /private device error/);
  f.harness.unmount();
});

test("a failed paid reservation retries the same automatic default without exposing alternatives", async () => {
  const f = setup(creditFailure);
  f.query.options.data.options.push({
    ...source,
    connection_id: sessionId,
    key_source: "own",
    default_model: false,
  });
  f.harness.render();
  assert.equal(
    f.all().some((row) => row.props.label === "Choose another voice source"),
    false,
  );
  await f.button("Try again").props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(f.events[0]?.[1])), saved);
  f.harness.unmount();
});

test("other startup errors also avoid claiming the call is ready or connected", () => {
  const f = setup({ error: "Microphone permission was denied" });
  assert.match(f.text(), /Unable to start/);
  assert.doesNotMatch(f.text(), /Ready to talk|Connected to this conversation/);
  assert.equal(
    f.all().some((row) => row.props.label === "Review billing on Web"),
    false,
  );
  f.harness.unmount();
});

function trackTranscript(f: ReturnType<typeof setup>) {
  const movements: unknown[][] = [];
  const scroll = () =>
    f.all().find((row) => row.type === "ScrollView" && row.props.ref)!;
  scroll().props.ref.current = {
    scrollTo: (options: unknown) =>
      movements.push(["top", JSON.parse(JSON.stringify(options))]),
    scrollToEnd: (options: unknown) =>
      movements.push(["end", JSON.parse(JSON.stringify(options))]),
  };
  return {
    movements,
    content: () => scroll().props.onContentSizeChange(360, 500),
  };
}

test("startup errors stay readable at the top instead of following the pricing footer", () => {
  const f = setup({
    error:
      "Voice ended while the app was inactive. Start a new call to continue.",
  });
  const transcript = trackTranscript(f);
  transcript.content();
  assert.deepEqual(transcript.movements, [["top", { y: 0, animated: false }]]);
  assert.equal(f.button("Try again").props.disabled, false);
  assert.ok(f.button("Close voice"));
  f.harness.unmount();
});

test("live caption previews do not scroll the voice controls", () => {
  const f = setup({ active: true, reducedMotion: true });
  const transcript = trackTranscript(f);
  transcript.content();
  assert.deepEqual(transcript.movements, []);
  f.voice.snapshot = {
    ...snapshot,
    captions: [
      {
        id: "caption-1",
        speaker: "user",
        text: "Hello",
        sealed: false,
        complete: false,
      },
    ],
  };
  f.harness.render();
  transcript.content();
  assert.deepEqual(transcript.movements, []);
  assert.match(f.text(), /Hello/);
  f.voice.connected = false;
  f.harness.render();
  transcript.content();
  assert.equal(transcript.movements.length, 0);
  f.harness.unmount();
});

test("an error takes precedence over following existing live captions", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      captions: [
        {
          id: "caption-1",
          speaker: "assistant",
          text: "Hello",
          sealed: true,
          complete: true,
        },
      ],
    },
  });
  const transcript = trackTranscript(f);
  f.voice.error = "Voice disconnected. Please try again.";
  f.harness.render();
  assert.deepEqual(transcript.movements, [["top", { y: 0, animated: false }]]);
  transcript.movements.length = 0;
  transcript.content();
  assert.deepEqual(transcript.movements, [["top", { y: 0, animated: false }]]);
  f.harness.unmount();
});

test("hidden settings errors do not scroll a connected call's controls", () => {
  const f = setup({
    active: true,
    reducedMotion: true,
    snapshot: {
      ...snapshot,
      captions: [
        {
          id: "caption-1",
          speaker: "assistant",
          text: "Hello",
          sealed: false,
          complete: false,
        },
      ],
    },
  });
  const transcript = trackTranscript(f);
  f.query.settings.error = new Error("The settings refresh failed.");
  f.harness.render();
  transcript.content();
  assert.deepEqual(transcript.movements, []);
  assert.equal(
    f.all().some((row) => row.props.message === "The settings refresh failed."),
    false,
  );
  f.harness.unmount();
});

test("replacing a visible error returns to its beginning even without a content-size event", () => {
  const f = setup({ error: "Call ended. Try again." });
  const transcript = trackTranscript(f);
  transcript.content();
  transcript.movements.length = 0;
  // Equal-length replacement text can keep the same native layout height.
  f.voice.error = "Call broke. Try again.";
  f.harness.render();
  assert.deepEqual(transcript.movements, [["top", { y: 0, animated: false }]]);
  transcript.movements.length = 0;
  f.harness.render();
  assert.deepEqual(transcript.movements, []);
  f.harness.unmount();
});

test("Grok forces Hold to talk even when the saved microphone mode was automatic", async () => {
  const grok: VoiceOption = {
    ...source,
    voice: {
      ...source.voice,
      protocol: "xai_realtime",
      usage_source: "server_measured",
    },
  };
  const f = setup({ source: grok });
  await Promise.resolve();
  assert.equal(
    (f.events[0]?.[1] as VoicePreferences).input_mode,
    "push_to_talk",
  );
  assert.equal(f.events[0]?.[2], "xai_realtime");
  assert.match(f.text(), /Hold to talk/);
  assert.doesNotMatch(f.text(), /Hands-free/);
  f.harness.unmount();
});

test("unavailable defaults cannot start and display clear guidance", () => {
  const f = setup({ source: { ...source, available: false } });
  assert.equal(f.button("Voice unavailable").props.disabled, true);
  assert.deepEqual(f.events, []);
  f.harness.unmount();
});

test("voice never exposes source or configuration selectors, and starts with the available default", async () => {
  const f = setup();
  f.query.options.data.options.unshift({
    ...source,
    model: "old",
    model_label: "Old choice",
    default_model: false,
  });
  f.query.options.data.options.push({
    ...source,
    key_source: "own",
    connection_id: sessionId,
    default_model: false,
  });
  f.harness.render();
  assert.equal(
    f
      .all()
      .some((row) => /source|model/i.test(row.props.accessibilityLabel ?? "")),
    false,
  );
  assert.doesNotMatch(
    JSON.stringify(f.harness.value()),
    /GPT Live|Old choice|Source & model/,
  );
  await Promise.resolve();
  assert.equal(
    (f.events.find((event) => event[0] === "start")?.[1] as VoicePreferences)
      ?.model,
    source.model,
  );
  f.harness.unmount();
});

test("closing voice releases push-to-talk and ends the call", async () => {
  const f = setup({ active: true });
  f.voice.holding = true;
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["hold", false], ["end"], ["close"]]);
  f.harness.unmount();
});

test("an approval review and task Stop remain separate from ending voice", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      tasks: [
        {
          id: serviceId,
          turn_id: "turn-1",
          state: "claimed",
          pending_acknowledgement_ids: [],
          title: "Read messages",
        },
        {
          id: sessionId,
          turn_id: "turn-2",
          state: "awaiting_confirmation",
          pending_acknowledgement_ids: ["ack-1"],
          title: "Send reply",
        },
        {
          id: "33333333-3333-4333-8333-333333333333",
          turn_id: "turn-3",
          state: "cancelled",
          outcome: "failed",
          pending_acknowledgement_ids: [],
          title: "Earlier failed request",
        },
      ],
    },
  });
  assert.match(f.text(), /Needs your OK/);
  assert.equal(f.find("Stop task"), undefined);
  f.button("Show agent tasks").props.onPress();
  assert.match(f.text(), /Tasks can continue after the call ends/);
  assert.equal(
    f.all().filter((row) => row.props.label === "Stop task").length,
    1,
  );
  assert.doesNotMatch(f.text(), /Earlier failed request/);
  f.voice.holding = true;
  f.button("Review in chat").props.onPress();
  f.button("Stop task").props.onPress();
  assert.deepEqual(f.events, [
    ["hold", false],
    ["review", "ack-1"],
    ["stopTask", serviceId],
  ]);
  assert.equal(
    f.events.some(([event]) => event === "end"),
    false,
  );
  f.harness.unmount();
});

test("failed task activity keeps request text and partial-effect warnings behind Details", () => {
  const request = "根据可用能力回答用户关于语音字幕的问题。";
  const warning =
    "The task failed before returning an answer. Some actions may already have finished; check them before retrying.";
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      tasks: [
        {
          id: serviceId,
          turn_id: "turn-1",
          state: "cancelled",
          outcome: "failed",
          pending_acknowledgement_ids: [],
          title: request,
          progress: {
            stage: "ended",
            detail: warning,
            last_completed_step: null,
            completed_steps: 0,
            updated_at: "2026-10-16T04:00:01Z",
          },
        },
      ],
    },
  });
  assert.match(f.text(), /1 task failed/);
  assert.doesNotMatch(f.text(), new RegExp(request));
  assert.doesNotMatch(f.text(), /Some actions may already have finished/);
  f.button("Show agent tasks").props.onPress();
  assert.match(f.text(), /Task 1 · Failed/);
  assert.doesNotMatch(
    f.text(),
    /Cancelled|Tasks can continue after the call ends/,
  );
  assert.doesNotMatch(f.text(), new RegExp(request));
  assert.doesNotMatch(f.text(), /Some actions may already have finished/);

  f.button("Show details for task 1").props.onPress();
  assert.match(f.text(), /Request/);
  assert.ok(f.text().includes(request));
  assert.ok(f.text().includes(warning));
  assert.ok(f.find("Hide details for task 1"));
  assert.deepEqual(f.events, []);
  f.button("Hide details for task 1").props.onPress();
  assert.doesNotMatch(f.text(), new RegExp(request));
  assert.doesNotMatch(f.text(), /Some actions may already have finished/);
  assert.deepEqual(f.events, []);
  f.harness.unmount();
});

test("terminal task activity distinguishes failure, Stop and an unknown legacy ending", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      tasks: [
        {
          id: serviceId,
          turn_id: "turn-1",
          state: "cancelled",
          outcome: "failed",
          pending_acknowledgement_ids: [],
        },
        {
          id: sessionId,
          turn_id: "turn-2",
          state: "cancelled",
          outcome: "cancelled",
          pending_acknowledgement_ids: [],
        },
        {
          id: "33333333-3333-4333-8333-333333333333",
          turn_id: "turn-3",
          state: "cancelled",
          pending_acknowledgement_ids: [],
        },
        {
          id: "44444444-4444-4444-8444-444444444444",
          turn_id: "turn-4",
          state: "completed",
          pending_acknowledgement_ids: [],
        },
      ],
    },
  });
  assert.match(f.text(), /1 task failed/);
  assert.doesNotMatch(f.text(), /in progress/);
  f.button("Show agent tasks").props.onPress();
  assert.match(f.text(), /Task 1 · Failed/);
  assert.match(f.text(), /Task 2 · Stopped/);
  assert.match(f.text(), /Task 3 · Ended/);
  assert.match(f.text(), /Task 4 · Done/);
  assert.doesNotMatch(
    f.text(),
    /Cancelled|Tasks can continue after the call ends/,
  );
  assert.equal(f.find("Stop task"), undefined);
  assert.equal(f.find("Review in chat"), undefined);
  assert.deepEqual(f.events, []);
  f.harness.unmount();
});

test("only active work contributes to the in-progress task count and stays stoppable", () => {
  for (const state of ["queued", "claimed"] as const) {
    const f = setup({
      active: true,
      snapshot: {
        ...snapshot,
        tasks: [
          {
            id: serviceId,
            turn_id: "turn-1",
            state,
            pending_acknowledgement_ids: [],
            title: "Current request",
          },
          {
            id: sessionId,
            turn_id: "turn-2",
            state: "cancelled",
            outcome: "failed",
            pending_acknowledgement_ids: [],
          },
          {
            id: "33333333-3333-4333-8333-333333333333",
            turn_id: "turn-3",
            state: "completed",
            pending_acknowledgement_ids: [],
          },
        ],
      },
    });
    assert.match(f.text(), /1 task in progress/);
    assert.doesNotMatch(f.text(), /3 tasks in progress/);
    f.button("Show agent tasks").props.onPress();
    assert.match(f.text(), /Tasks can continue after the call ends/);
    f.button("Stop task").props.onPress();
    assert.deepEqual(f.events, [["stopTask", serviceId]]);
    assert.ok(f.find("End voice and close"));
    f.harness.unmount();
  }
});

test("details reveal only the selected request and never perform a task operation", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      tasks: [
        {
          id: serviceId,
          turn_id: "turn-1",
          state: "completed",
          pending_acknowledgement_ids: [],
          title: "First private request",
        },
        {
          id: sessionId,
          turn_id: "turn-2",
          state: "cancelled",
          outcome: "cancelled",
          pending_acknowledgement_ids: [],
          title: "Second private request",
        },
      ],
    },
  });
  assert.match(f.text(), /Task activity/);
  assert.doesNotMatch(f.text(), /in progress|task failed/);
  f.button("Show agent tasks").props.onPress();
  f.button("Show details for task 1").props.onPress();
  assert.match(f.text(), /First private request/);
  assert.doesNotMatch(f.text(), /Second private request/);
  f.button("Hide details for task 1").props.onPress();
  f.button("Show details for task 2").props.onPress();
  assert.match(f.text(), /Second private request/);
  assert.doesNotMatch(f.text(), /First private request/);
  assert.deepEqual(f.events, []);
  f.harness.unmount();
});

test("opening task activity and its details releases held PTT without ending the call", () => {
  const f = setup({
    active: true,
    saved: { ...saved, input_mode: "push_to_talk" },
    snapshot: {
      ...snapshot,
      tasks: [
        {
          id: serviceId,
          turn_id: "turn-1",
          state: "claimed",
          pending_acknowledgement_ids: [],
          title: "Current request",
        },
      ],
    },
  });
  const bridge = () => f.all().find((row) => row.type === "VoiceMediaBridge")!;
  const original = bridge();
  f.voice.holding = true;
  f.button("Show agent tasks").props.onPress();
  assert.equal(f.voice.holding, false);
  f.voice.holding = true;
  f.button("Show details for task 1").props.onPress();
  assert.equal(f.voice.holding, false);
  assert.deepEqual(f.events, [
    ["hold", false],
    ["hold", false],
  ]);
  assert.equal(bridge().key, original.key);
  assert.equal(bridge().props.media, original.props.media);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("task settlement updates activity without restarting media or retaining a running disclaimer", () => {
  const pending: VoiceSnapshot = {
    ...snapshot,
    tasks: [
      {
        id: serviceId,
        turn_id: "turn-1",
        state: "claimed",
        pending_acknowledgement_ids: [],
        title: "A request",
      },
    ],
  };
  const f = setup({ active: true, snapshot: pending });
  const bridge = () => f.all().find((row) => row.type === "VoiceMediaBridge")!;
  const original = bridge();
  f.button("Show agent tasks").props.onPress();
  assert.match(f.text(), /Tasks can continue after the call ends/);
  const pendingTask = pending.tasks[0];
  assert.ok(pendingTask);
  f.voice.snapshot = {
    ...pending,
    tasks: [{ ...pendingTask, state: "cancelled", outcome: "failed" }],
  };
  f.harness.render();
  assert.match(f.text(), /1 task failed/);
  assert.match(f.text(), /Task 1 · Failed/);
  assert.doesNotMatch(
    f.text(),
    /in progress|Tasks can continue after the call ends/,
  );
  assert.equal(f.find("Stop task"), undefined);
  assert.equal(bridge().key, original.key);
  assert.equal(bridge().props.media, original.props.media);
  assert.deepEqual(f.events, []);
  f.harness.unmount();
});

test("approval-only task activity asks for card review without offering Stop or a running disclaimer", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      tasks: [
        {
          id: serviceId,
          turn_id: "turn-1",
          state: "awaiting_confirmation",
          pending_acknowledgement_ids: ["ack-1"],
        },
      ],
    },
  });
  f.button("Show agent tasks").props.onPress();
  assert.match(f.text(), /Review the pending request in chat to continue/);
  assert.doesNotMatch(f.text(), /Tasks can continue after the call ends/);
  assert.equal(f.find("Stop task"), undefined);
  f.button("Review in chat").props.onPress();
  assert.deepEqual(f.events, [["review", "ack-1"]]);
  f.harness.unmount();
});

test("task details stay attached to the request when snapshots reorder or remove it", () => {
  const first: VoiceSnapshot["tasks"][number] = {
    id: serviceId,
    turn_id: "turn-1",
    state: "completed",
    pending_acknowledgement_ids: [],
    title: "First request text",
  };
  const second: VoiceSnapshot["tasks"][number] = {
    id: sessionId,
    turn_id: "turn-2",
    state: "completed",
    pending_acknowledgement_ids: [],
    title: "Second request text",
  };
  const f = setup({
    active: true,
    snapshot: { ...snapshot, tasks: [first, second] },
  });
  f.button("Show agent tasks").props.onPress();
  f.button("Show details for task 1").props.onPress();
  f.voice.snapshot = { ...snapshot, tasks: [second, first] };
  f.harness.render();
  assert.match(f.text(), /First request text/);
  assert.doesNotMatch(f.text(), /Second request text/);
  assert.ok(f.find("Hide details for task 2"));
  f.voice.snapshot = { ...snapshot, tasks: [second] };
  f.harness.render();
  assert.doesNotMatch(f.text(), /First request text|Second request text/);
  f.voice.snapshot = { ...snapshot, tasks: [] };
  f.harness.render();
  assert.equal(f.find("Agent tasks"), undefined);
  assert.equal(f.find("Show agent tasks"), undefined);
  assert.equal(f.find("Hide agent tasks"), undefined);
  assert.deepEqual(f.events, []);
  f.harness.unmount();
});

test("a new call gets a fresh media engine while ordinary updates preserve its owner", () => {
  const f = setup({ active: true });
  const first = f.media();
  f.voice.snapshot = {
    ...snapshot,
    captions: [
      {
        id: "caption-1",
        speaker: "user",
        text: "你好",
        sealed: true,
        complete: true,
      },
    ],
  };
  f.harness.render();
  assert.equal(f.media().key, first.key);
  assert.equal(f.media().props.media, first.props.media);
  f.voice.media = { ...f.voice.media!, id: 2 };
  f.voice.mediaAttempt = 2;
  f.harness.render();
  assert.notEqual(f.media().key, first.key);
  assert.notEqual(f.media().props.media, first.props.media);
  f.harness.unmount();
});

test("closing voice before a call returns to the message composer", () => {
  const f = setup();
  f.button("Close voice").props.onPress();
  // Nothing is connected or retained, so there is no call to End.
  assert.deepEqual(
    f.events.map((e) => e[0]),
    ["start", "close"],
  );
  f.harness.unmount();
});

test("closing voice ends the call and leaves accepted tasks running", async () => {
  const f = setup({ active: true });
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  f.harness.unmount();
});

test("push-to-talk disables input during server muting and exposes accessible press controls", () => {
  const f = setup({
    active: true,
    saved: { ...saved, input_mode: "push_to_talk" },
    snapshot: {
      ...snapshot,
      session: { ...snapshot.session, input_muted: true },
    },
  });
  const hold = f.button("Hold to talk");
  assert.equal(hold.props.disabled, true);
  assert.equal(typeof hold.props.onPressIn, "function");
  assert.equal(typeof hold.props.onPressOut, "function");
  assert.equal(typeof hold.props.onAccessibilityAction, "function");
  f.harness.unmount();
});

test("both listening modes time the connected call, and Hold to talk keeps its explicit microphone controls", () => {
  const walkie = setup({
    active: true,
    saved: { ...saved, input_mode: "push_to_talk" },
  });
  const labels = walkie.all().map((row) => row.props?.accessibilityLabel);
  assert.equal(
    labels.some((label) => String(label ?? "").startsWith("Call time")),
    true,
    "connected call time is independent of listening mode",
  );
  const nodes = walkie.all();
  const hold = nodes.findIndex(
    (row) => row.props?.accessibilityLabel === "Hold to talk",
  );
  const hint = nodes.findIndex(
    (row, index) =>
      index > hold &&
      row.type === "Text" &&
      [row.props.children].flat().includes("Hold to talk"),
  );
  assert.ok(hold >= 0 && hint > hold, "the instruction follows the microphone");
  const mute = nodes.find(
    (row) => row.props?.accessibilityLabel === "Mute microphone",
  )!;
  assert.ok(
    [mute.props.children]
      .flat(Infinity)
      .some((child: any) => child?.type === "MicOff"),
    "the left control is a mute symbol, not a second mic",
  );
  assert.doesNotMatch(
    walkie.text(),
    /Audio is sent only/,
    "details stay collapsed",
  );
  walkie.harness.unmount();
  const phone = setup({ active: true });
  assert.ok(
    phone
      .all()
      .some((row) =>
        String(row.props?.accessibilityLabel ?? "").startsWith("Call time"),
      ),
  );
  assert.ok(
    phone.all().some((row) => row.type === "PhoneOff"),
    "End hangs up",
  );
  assert.doesNotMatch(
    phone.text(),
    /Talk any time|Resume audio/,
    "no helpers once connected",
  );
  phone.button("Voice help").props.onPress();
  phone.harness.render();
  assert.ok(phone.button("Resume audio"), "recovery lives in the help popover");
  phone.harness.unmount();
});

test("loss of connectivity ends local voice and disables starting until the app reconnects", () => {
  const live = setup({ active: true });
  live.setOnline(false);
  assert.deepEqual(live.events, [["end"]]);
  live.harness.unmount();
  const f = setup();
  f.setOnline(false);
  assert.equal(f.button("Try again").props.disabled, true);
  // Without a connected call there is nothing to End.
  assert.deepEqual(
    f.events.map((e) => e[0]),
    ["start"],
  );
  assert.equal(
    f
      .all()
      .some(
        (row) => row.type === "Notice" && /offline/i.test(row.props.message),
      ),
    true,
  );
  f.harness.unmount();
});

test("the voice composer's animated bubble follows listening, speaking and thinking", () => {
  const f = setup({ active: true });
  assert.equal(f.levels.media, f.voice.media);
  assert.equal(f.aura().phase, "listening");
  assert.equal(f.aura().input, f.levels.input);
  assert.equal(f.aura().output, f.levels.output);
  assert.equal(f.aura().reducedMotion, false);
  assert.match(f.text(), /Listening/);
  f.levels.speaking = true;
  f.harness.render();
  assert.equal(f.aura().phase, "speaking");
  assert.match(f.text(), /Speaking/);
  f.levels.speaking = false;
  f.voice.snapshot = {
    ...snapshot,
    captions: [
      {
        id: "caption-1",
        speaker: "user",
        text: "Summarise my inbox",
        sealed: true,
        complete: true,
      },
    ],
  };
  f.harness.render();
  assert.equal(f.aura().phase, "thinking");
  assert.match(f.text(), /Thinking/);
  f.harness.unmount();
});

test("Reduce Motion reaches the aura", () => {
  const f = setup({ active: true, reducedMotion: true });
  assert.equal(f.aura().reducedMotion, true);
  f.harness.unmount();
});

test("round call controls mute the microphone and speaker and End closes the call", async () => {
  const f = setup({
    active: true,
    saved: { ...saved, input_mode: "push_to_talk" },
  });
  f.button("Mute microphone").props.onPress();
  f.button("Mute speaker").props.onPress();
  const hold = f.button("Hold to talk");
  hold.props.onPressIn();
  hold.props.onPressOut();
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [
    ["mute", true],
    ["speaker", true],
    ["hold", true],
    ["hold", false],
    ["end"],
    ["close"],
  ]);
  f.harness.unmount();
});

test("automatic mode has no hold button and the call shows no in-call pickers", () => {
  const f = setup({ active: true });
  assert.equal(
    f.all().some((row) => row.props.accessibilityLabel === "Hold to talk"),
    false,
  );
  assert.equal(
    f
      .all()
      .some((row) =>
        /^(Voice|Microphone):/.test(row.props.accessibilityLabel ?? ""),
      ),
    false,
  );
  f.harness.unmount();
});

test("call startup shows its mode and status without extra greeting or settings guidance", () => {
  const f = setup();
  assert.doesNotMatch(
    f.text(),
    /Hey, what can I do for you today|Settings › Voice/,
  );
  assert.deepEqual(JSON.parse(JSON.stringify(f.starts()[0]?.[1])), saved);
  assert.match(f.text(), /Hands-free/);
  assert.match(f.text(), /Connecting/);
  assert.equal(f.aura().phase, "resting");
  f.harness.unmount();
});

test("only unsealed live captions appear in the voice composer", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      captions: ["one", "two", "three", "agent still talking"].map(
        (text, index) => ({
          id: `caption-${index}`,
          speaker: index % 2 ? ("assistant" as const) : ("user" as const),
          text,
          sealed: index < 2,
          complete: index < 2,
        }),
      ),
    },
  });
  const text = f.text();
  assert.doesNotMatch(text, /\bone\b|\btwo\b/);
  assert.match(text, /three/);
  assert.doesNotMatch(text, /Show all|Show latest/);
  f.harness.unmount();
});

test("a call without tasks shows no tasks tray", () => {
  const f = setup({ active: true });
  assert.doesNotMatch(f.text(), /Agent tasks/);
  f.harness.unmount();
});

test("voice is a composer with the animated bubble on top, not a page over the chat", async () => {
  const idle = setup();
  assert.equal(
    idle.all().some((row) => row.type === "Modal" && row.props.visible),
    false,
  );
  const frame = idle.all().find((row) => row.type === "ComposerFrame")!;
  assert.ok(frame, "it takes the message composer's place");
  assert.ok(
    idle.all().some((row) => row.type === "VoiceAura"),
    "the aura around the composer",
  );
  assert.match(idle.text(), /我的助手/);
  assert.equal(idle.events[0]?.[0], "start");
  idle.harness.unmount();
  // While options load, the centre control says it is connecting.
  const loading = setup({ settingsPending: true });
  assert.equal(loading.button("Connecting voice").props.disabled, true);
  assert.equal(loading.starts().length, 0);
  loading.harness.unmount();
  // Hands-free: the microphone in the middle, End on the right.
  const f = setup({ active: true });
  f.button("Mute microphone").props.onPress();
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["mute", true], ["end"], ["close"]]);
  f.harness.unmount();
});

test("the call switches between Hands-free and Hold to talk in place and remembers it", async () => {
  const f = setup({ active: true });
  assert.equal(
    f.button("Hands-free mode").props.accessibilityState.selected,
    true,
  );
  f.button("Hold to talk mode").props.onPress();
  f.harness.render();
  await f.harness.settle();
  assert.deepEqual(f.events.slice(0, 2), [
    ["mode", "push_to_talk"],
    ["save", "push_to_talk"],
  ]);
  // Hold to talk is now the centre control and the toggle reflects it.
  assert.equal(typeof f.button("Hold to talk").props.onPressIn, "function");
  f.harness.unmount();
  // Before a call the choice is only saved, for the call about to start.
  const idle = setup();
  idle.button("Hold to talk mode").props.onPress();
  await idle.harness.settle();
  assert.deepEqual(
    idle.events.map((e) => e[0]),
    ["start", "save", "settingsRefetch"],
  );
  assert.deepEqual(
    idle.events.find((e) => e[0] === "save"),
    ["save", "push_to_talk"],
  );
  idle.harness.unmount();
});

test("Grok keeps Hold to talk and offers no mode toggle", () => {
  const f = setup({
    active: true,
    source: {
      ...source,
      voice: {
        ...source.voice,
        protocol: "xai_realtime",
        usage_source: "server_measured",
      },
    },
  });
  assert.equal(
    f.all().some((row) => row.props.accessibilityLabel === "Hands-free mode"),
    false,
  );
  assert.equal(typeof f.button("Hold to talk").props.onPressIn, "function");
  f.harness.unmount();
});

test("entering voice auto-starts once and never silently retries a failed or ended call", () => {
  const f = setup();
  assert.equal(f.events.filter((e) => e[0] === "start").length, 1);
  f.harness.render();
  f.harness.render();
  assert.equal(f.events.filter((e) => e[0] === "start").length, 1);
  f.voice.error = "Microphone denied";
  f.harness.render();
  assert.equal(f.events.filter((e) => e[0] === "start").length, 1);
  f.button("Try again").props.onPress();
  assert.equal(f.events.filter((e) => e[0] === "start").length, 2);
  f.harness.unmount();
});

test("connecting displays a placeholder and the active call starts at 00:00 regardless of creation time", () => {
  const f = setup();
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Call time -"));
  f.voice.starting = true;
  f.harness.render();
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Call time -"));
  f.voice.starting = false;
  f.voice.connected = true;
  f.voice.connectedAt = Date.now();
  f.voice.snapshot = snapshot;
  f.harness.render();
  assert.ok(
    f.all().some((n) => n.props.accessibilityLabel === "Call time 00:00"),
  );
  f.harness.unmount();
});

test("voice is speech-only in both sizes, and row expansion preserves the media and session", () => {
  const f = setup({ active: true });
  const media = f.media();
  assert.equal(
    f.all().some((n) => n.type === "TextInput"),
    false,
  );
  assert.ok(f.all().some((n) => n.type === "DictationWaveform"));
  f.props.minimized = true;
  f.harness.render();
  assert.equal(f.media().key, media.key);
  assert.equal(f.media().props.media, media.props.media);
  assert.equal(f.events.length, 0);
  assert.equal(
    f.all().some((n) => n.type === "TextInput"),
    false,
  );
  assert.ok(f.all().some((n) => n.type === "DictationWaveform"));
  assert.ok(f.button("Expand voice composer").props.style.minWidth >= 44);
  assert.ok(f.button("Expand voice composer").props.style.minHeight >= 44);
  f.button("Expand voice composer").props.onPress();
  assert.deepEqual(f.events, [["expand"]]);
  f.props.minimized = false;
  f.harness.render();
  assert.equal(f.media().key, media.key);
  assert.equal(
    f.events.some((e) => e[0] === "end" || e[0] === "start"),
    false,
  );
  f.harness.unmount();
});

for (const state of [
  "connecting",
  "connected",
  "idle",
  "options",
  "minimized",
]) {
  test(`pricing is absent in ${state} voice`, () => {
    const f = setup({
      active:
        state === "connected" || state === "options" || state === "minimized",
      source: {
        ...source,
        pricing: {
          metric: "voice_seconds",
          credits_per_unit: "0.025",
          sync_status: "synced",
          components: [],
        } as any,
      },
    });
    if (state === "connecting") {
      f.voice.starting = true;
      f.harness.render();
    }
    if (state === "options") f.button("Voice help").props.onPress();
    if (state === "minimized") {
      f.props.minimized = true;
      f.harness.render();
    }
    f.harness.flush();
    assert.doesNotMatch(f.text(), /credits|tariff|0\.025|per second/i);
    assert.equal(
      f.all().some((n) => n.props.accessibilityLabel === "Voice tariff"),
      false,
    );
    f.harness.unmount();
  });
}

for (const viewport of [
  { width: 375, height: 667, fontScale: 1 },
  { width: 440, height: 956, fontScale: 1 },
  { width: 834, height: 1194, fontScale: 1 },
  { width: 375, height: 834, fontScale: 2.5 },
]) {
  test(`minimized voice shares normal composer insets and single-line geometry at ${viewport.width}pt/${viewport.fontScale} text scale`, () => {
    const f = setup({ active: true, viewport });
    f.props.minimized = true;
    f.harness.render();
    const row = f.all().find((n) => n.props.testID === "voice-compact-row")!;
    assert.equal(
      row.props.style[1].minHeight,
      composerVoiceMinHeight(viewport.fontScale),
    );
    const frame = f.all().find((n) => n.type === "ComposerFrame")!;
    assert.equal(frame.props.style.paddingHorizontal, 4);
    assert.equal(frame.props.style.paddingVertical, 4);
    const end = f.button("End voice and close");
    assert.ok(end.props.style({ pressed: false })[0].width >= 44);
    assert.equal(row.props.style[0].alignItems, "center");
    f.harness.unmount();
  });
}

for (const minimized of [false, true]) {
  for (const reducedMotion of [false, true]) {
    test(`hold to talk transmits on press/release in ${minimized ? "minimized" : "expanded"} voice with Reduce Motion ${reducedMotion}`, () => {
      const f = setup({
        active: true,
        reducedMotion,
        saved: { ...saved, input_mode: "push_to_talk" },
      });
      f.props.minimized = minimized;
      f.harness.render();
      const button = f.button("Hold to talk");
      assert.equal(button.props.disabled, false);
      assert.equal(f.aura().phase, "resting");
      assert.match(f.text(), /Hold to talk/);
      assert.ok(f.all().some((n) => n.props.testID === "voice-hold-halo"));
      button.props.onPressIn();
      f.harness.render();
      assert.equal(
        f.voice.holding,
        true,
        "compact PTT must not be cancelled by the minimized effect",
      );
      assert.equal(f.aura().phase, "listening");
      assert.equal(
        f.button("Hold to talk").props.accessibilityState.selected,
        true,
      );
      assert.equal(
        f.button("Hold to talk").props.accessibilityHint,
        "Release to send",
      );
      assert.match(f.text(), /Release to send/);
      f.button("Hold to talk").props.onPressOut();
      f.harness.render();
      assert.equal(f.voice.holding, false);
      assert.equal(f.aura().phase, "resting");
      assert.deepEqual(f.events, [
        ["hold", true],
        ["hold", false],
      ]);
      assert.equal(
        f.all().some((n) => n.type === "TextInput"),
        false,
      );
      f.harness.unmount();
    });
  }
}

test("changing size releases an existing hold and retains the listening mode", () => {
  const f = setup({ active: true });
  f.button("Hold to talk mode").props.onPress();
  f.harness.render();
  f.button("Hold to talk").props.onPressIn();
  f.harness.render();
  f.props.minimized = true;
  f.harness.render();
  assert.equal(f.voice.holding, false);
  f.button("Hold to talk").props.onPressIn();
  f.harness.render();
  assert.equal(f.voice.holding, true);
  f.button("Hold to talk").props.onPressOut();
  f.button("Expand voice composer").props.onPress();
  f.props.minimized = false;
  f.harness.render();
  assert.equal(
    f.button("Hold to talk mode").props.accessibilityState.selected,
    true,
  );
  f.button("Hands-free mode").props.onPress();
  f.harness.render();
  assert.ok(f.button("Mute microphone"));
  assert.equal(
    f.all().some((n) => n.props.accessibilityLabel === "Hold to talk"),
    false,
  );
  f.harness.unmount();
});

for (const mode of ["automatic", "push_to_talk"] as const) {
  test(`the compact voice help popover explains ${mode}, dismisses and resumes audio without pricing`, async () => {
    const f = setup({ active: true, saved: { ...saved, input_mode: mode } });
    assert.equal(
      f.all().some((n) => n.props.accessibilityLabel === "Call options"),
      false,
    );
    const help = f.button("Voice help");
    assert.ok(help.props.style({ pressed: false })[0].width >= 44);
    help.props.onPress();
    f.harness.render();
    assert.equal(f.modal().visible, true);
    assert.match(
      f.text(),
      mode === "automatic" ? /answers when you pause/ : /Release to send/,
    );
    assert.match(f.text(), /headphones if you hear echo/);
    assert.doesNotMatch(f.text(), /tariff|credits per|0\.025/i);
    f.button("Close voice help").props.onPress();
    f.harness.render();
    assert.equal(
      f.all().some((n) => n.type === "Modal"),
      false,
    );
    f.button("Voice help").props.onPress();
    f.harness.render();
    f.button("Dismiss voice help").props.onPress();
    f.harness.render();
    assert.equal(
      f.all().some((n) => n.type === "Modal"),
      false,
    );
    f.button("Voice help").props.onPress();
    f.harness.render();
    f.modal().onRequestClose();
    f.harness.render();
    assert.equal(
      f.all().some((n) => n.type === "Modal"),
      false,
    );
    f.button("Voice help").props.onPress();
    f.harness.render();
    await f.button("Resume audio").props.onPress();
    f.harness.render();
    assert.deepEqual(f.events, [["resume"]]);
    assert.equal(
      f.all().some((n) => n.type === "Modal"),
      false,
    );
    f.button("Voice help").props.onPress();
    f.harness.render();
    f.props.minimized = true;
    f.harness.render();
    assert.equal(
      f.all().some((n) => n.type === "Modal"),
      false,
    );
    f.harness.unmount();
  });
}

for (const reducedMotion of [false, true]) {
  test(`changing listening mode animates its header width unless Reduce Motion is ${reducedMotion}`, () => {
    const f = setup({ active: true, reducedMotion });
    f.button("Hold to talk mode").props.onPress();
    f.harness.render();
    assert.deepEqual(
      f.layoutAnimations,
      reducedMotion ? [] : ["easeInEaseOut"],
    );
    f.harness.unmount();
  });
}

for (const width of [375, 440, 834]) {
  test(`the ${width}pt header mode toggle labels only its active radio, keeps 44pt targets and saves preferences`, async () => {
    const f = setup({
      active: true,
      viewport: { width, height: 834, fontScale: 2.5 },
    });
    const radios = () =>
      f.all().filter((n) => n.props.accessibilityRole === "radio");
    assert.equal(radios().length, 2);
    for (const radio of radios()) {
      assert.ok(radio.props.style[0].minWidth >= 44);
      assert.ok(radio.props.style[0].minHeight >= 44);
    }
    const children = (label: string) => elements(f.button(label));
    assert.ok(children("Hands-free mode").some((n) => n.type === "Phone"));
    assert.ok(children("Hands-free mode").some((n) => n.type === "Text"));
    assert.ok(children("Hold to talk mode").some((n) => n.type === "Radio"));
    assert.equal(
      children("Hold to talk mode").some((n) => n.type === "Text"),
      false,
    );
    f.button("Hold to talk mode").props.onPress();
    f.harness.render();
    await f.harness.settle();
    assert.equal(
      children("Hands-free mode").some((n) => n.type === "Text"),
      false,
    );
    assert.ok(children("Hold to talk mode").some((n) => n.type === "Text"));
    assert.deepEqual(f.events.slice(0, 2), [
      ["mode", "push_to_talk"],
      ["save", "push_to_talk"],
    ]);
    f.voice.starting = true;
    f.harness.render();
    assert.ok(radios().every((n) => n.props.disabled));
    f.harness.unmount();
  });
}

// Call lifecycle, recovery and preference behaviour from #2321.

test("the explicit voice entry automatically starts once with the exact saved source", async () => {
  const f = setup();
  assert.equal(f.starts().length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(f.starts()[0]?.[1])), saved);
  assert.equal(f.starts()[0]?.[2], "openai_live");
  f.harness.render();
  f.query.options.data = { options: [...f.query.options.data.options] };
  f.harness.render();
  await f.harness.settle();
  assert.equal(f.starts().length, 1);
  assert.equal(f.find("Start voice"), undefined);
  assert.doesNotMatch(f.text(), /Tap to start|Ready to talk/);
  f.harness.unmount();
});

test("auto-start waits for both options and saved settings", () => {
  for (const pending of ["settings", "options"] as const) {
    const f = setup({
      settingsPending: pending === "settings",
      optionsPending: pending === "options",
    });
    assert.equal(f.starts().length, 0);
    f.query.settings.data = { voice: saved };
    f.query.settings.isSuccess = true;
    f.query.settings.isPending = false;
    f.query.options.isSuccess = true;
    f.query.options.isPending = false;
    f.harness.render();
    assert.equal(f.starts().length, 1);
    assert.deepEqual(JSON.parse(JSON.stringify(f.starts()[0]?.[1])), saved);
    f.harness.unmount();
  }
});

test("first-use OpenAI defaults to automatic and a platform default without replacing saved PTT", () => {
  const first = setup({
    saved: null,
    options: [
      {
        ...source,
        default_model: false,
        key_source: "own",
        connection_id: sessionId,
      },
      source,
    ],
  });
  assert.equal(
    (first.starts()[0]?.[1] as VoicePreferences).input_mode,
    "automatic",
  );
  assert.equal(
    (first.starts()[0]?.[1] as VoicePreferences).key_source,
    "platform",
  );
  first.harness.unmount();
  const ptt = setup({ saved: { ...saved, input_mode: "push_to_talk" } });
  assert.equal(
    (ptt.starts()[0]?.[1] as VoicePreferences).input_mode,
    "push_to_talk",
  );
  ptt.harness.unmount();
});

test("a saved own source wins over an available platform source", () => {
  const own = {
    ...source,
    key_source: "own" as const,
    connection_id: sessionId,
    model: "saved-model",
    default_model: false,
  };
  const preferences = {
    ...saved,
    key_source: own.key_source,
    connection_id: own.connection_id,
    model: own.model,
  };
  const f = setup({ saved: preferences, options: [source, own] });
  assert.deepEqual(JSON.parse(JSON.stringify(f.starts()[0]?.[1])), preferences);
  f.harness.unmount();
});

test("missing sources, unavailable sources and missing saved voices never substitute or start", () => {
  for (const options of [
    [],
    [
      {
        ...source,
        available: false,
        unavailable_reason: "Reconnect saved provider",
      },
    ],
    [source],
  ]) {
    const f = setup({
      options,
      saved:
        options.length && options[0]?.available
          ? { ...saved, voice: "retired" }
          : saved,
    });
    assert.equal(f.starts().length, 0);
    assert.equal(f.find("Start voice"), undefined);
    assert.ok(f.notices().length);
    assert.deepEqual(f.savedPreferences, []);
    f.harness.unmount();
  }
});

test("an unavailable source is not presented as connecting or as a guessed supplier", () => {
  const f = setup({ options: [] });
  assert.match(f.text(), /Voice unavailable/);
  assert.doesNotMatch(f.text(), /Connecting/);
  f.openPreferences();
  assert.match(f.text(), /Voice source unavailable/);
  assert.doesNotMatch(f.text(), /OpenAI/);
  f.harness.unmount();
});

test("Grok stays on its saved supplier and always uses explicit Hold to talk", () => {
  const grok: VoiceOption = {
    ...source,
    voice: {
      ...source.voice,
      protocol: "xai_realtime",
      usage_source: "server_measured",
    },
  };
  const f = setup({ source: grok });
  assert.equal(f.starts()[0]?.[2], "xai_realtime");
  assert.equal(
    (f.starts()[0]?.[1] as VoicePreferences).input_mode,
    "push_to_talk",
  );
  f.openPreferences();
  assert.equal(f.find("Hands-free mode"), undefined);
  // The header toggle offers only Hold to talk, locked for this source.
  assert.equal(f.button("Hold to talk mode").props.disabled, true);
  assert.equal(
    f.button("Hold to talk mode").props.accessibilityState.selected,
    true,
  );
  f.harness.unmount();
});

test("a failed start requires an explicit retry and hides billing diagnostics by default", async () => {
  const f = setup(creditFailure);
  assert.equal(f.starts().length, 0);
  assert.match(f.text(), /Unable to start/);
  assert.ok(f.notices().includes(creditFailure.recovery.message));
  assert.equal(f.text().includes(creditFailure.error), false);
  f.harness.render();
  assert.equal(f.starts().length, 0);
  f.button("Show error details").props.onPress();
  assert.equal(f.text().includes(creditFailure.error), true);
  await f.button("Try again").props.onPress();
  assert.equal(f.starts().length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(f.starts()[0]?.[1])), saved);
  f.harness.unmount();
});

test("billing recovery opens the configured Web page and text recovery does not create another call", async () => {
  const f = setup(creditFailure);
  await f.button("Review billing on Web").props.onPress();
  f.button("Continue with text").props.onPress();
  assert.deepEqual(f.events, [
    ["billing", "https://nyx.example/billing"],
    ["close"],
  ]);
  f.harness.unmount();
});

test("billing browser failures keep the recovery error and sanitize platform details", async () => {
  const f = setup(creditFailure);
  f.failBrowser();
  await f.button("Review billing on Web").props.onPress();
  assert.match(f.notices().join(" "), /Billing could not open/);
  assert.doesNotMatch(f.notices().join(" "), /private browser error/);
  assert.match(f.text(), /Unable to start/);
  f.harness.unmount();
});

test("snapshots go to the one main timeline and are cleared on unmount", () => {
  const f = setup({ active: true });
  assert.equal(f.snapshots.at(-1), snapshot);
  f.voice.snapshot = {
    ...snapshot,
    captions: [
      {
        id: "segment",
        speaker: "assistant",
        text: "authoritative words",
        sealed: true,
        complete: false,
      },
    ],
  };
  f.harness.render();
  assert.equal(f.snapshots.at(-1), f.voice.snapshot);
  assert.doesNotMatch(f.text(), /authoritative words/);
  f.harness.unmount();
  assert.equal(f.snapshots.at(-1), undefined);
});

test("Listening follows actual input permission, Speaking follows playback, and captions do not invent audio activity", () => {
  const f = setup({
    active: true,
    inputEnabled: false,
    snapshot: {
      ...snapshot,
      captions: [
        {
          id: "old",
          speaker: "assistant",
          text: "old partial",
          complete: false,
          sealed: false,
        },
      ],
    },
  });
  assert.match(f.text(), /Connected/);
  assert.doesNotMatch(f.text(), /Listening|Speaking/);
  f.voice.inputEnabled = true;
  f.harness.render();
  assert.match(f.text(), /Listening/);
  f.levels.speaking = true;
  f.harness.render();
  assert.match(f.text(), /Speaking/);
  f.voice.speakerMuted = true;
  f.harness.render();
  assert.doesNotMatch(f.text(), /Speaking/);
  f.harness.unmount();
});

test("a verified quiet-call terminal state shows Close without the stale idle warning", () => {
  const f = setup({
    snapshot: {
      ...snapshot,
      session: {
        ...snapshot.session,
        state: "closed",
        closed_at: "2026-10-16T04:04:25Z",
        end_reason: "idle",
        end_requested: true,
        idle_warning: true,
      },
    },
  });
  assert.match(f.text(), /Call ended/);
  assert.doesNotMatch(f.text(), /Unable to start/);
  assert.deepEqual(f.notices(), []);
  assert.ok(f.button("Close voice"));
  assert.equal(f.find("End voice and close"), undefined);
  assert.equal(f.starts().length, 0);
  f.button("Close voice").props.onPress();
  assert.deepEqual(f.events, [["close"]]);
  f.harness.unmount();
});

for (const state of ["closed", "failed"] as const) {
  test(`a terminal ${state} conflict with end_requested offers Close without starting a replacement`, () => {
    const f = setup({
      conflict: { ...snapshot.session, state, end_requested: true },
    });
    assert.match(f.text(), /Call ended/);
    assert.doesNotMatch(f.text(), /Confirming|Call already active/);
    assert.ok(f.button("Close voice"));
    assert.equal(f.find("Request end again"), undefined);
    assert.equal(f.find("Check call status"), undefined);
    assert.equal(f.starts().length, 0);
    f.button("Close voice").props.onPress();
    assert.deepEqual(f.events, [["close"]]);
    f.harness.unmount();
  });
}

test("a terminal session supersedes its own pending cleanup without dismissing a different active call", () => {
  const f = setup({
    snapshot: {
      ...snapshot,
      session: { ...snapshot.session, state: "closed", end_requested: true },
    },
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
  });
  assert.match(f.text(), /Call ended/);
  assert.ok(f.button("Close voice"));
  assert.equal(f.find("Request end again"), undefined);
  f.voice.conflict = {
    ...snapshot.session,
    id: "33333333-3333-4333-8333-333333333333",
  };
  f.harness.render();
  assert.match(f.text(), /Call already active/);
  assert.ok(f.button("End existing call"));
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("voice changes are saved for next call without changing this call, while the mode toggle switches it in place", async () => {
  const f = setup({ active: true });
  f.openPreferences();
  assert.match(f.text(), /Changes apply to your next call/);
  await f.button("Hold to talk mode").props.onPress();
  await f.harness.settle();
  assert.deepEqual(
    f.savedPreferences.map((row) => row.input_mode),
    ["push_to_talk"],
  );
  // The listening mode applies to the live call too.
  assert.deepEqual(
    f.events.filter(([event]) => event === "mode"),
    [["mode", "push_to_talk"]],
  );
  assert.equal(typeof f.button("Hold to talk").props.onPressIn, "function");
  f.button("Voice").props.onPress();
  f.voicePicker()!.props.onChange(["marin"]);
  await f.harness.settle();
  assert.equal(f.savedPreferences.at(-1)?.voice, "marin");
  assert.equal(f.savedPreferences.at(-1)?.service_id, saved.service_id);
  assert.equal(f.savedPreferences.at(-1)?.input_mode, "push_to_talk");
  assert.equal(f.button("Voice").props.value, "Marin");
  assert.equal(f.starts().length, 0);
  assert.equal(
    f.events.filter(([event]) => event === "end" || event === "mode").length,
    1,
    "only the explicit mode switch reached the live call",
  );
  f.harness.unmount();
});

test("voice preferences show one saved-voice row while all server voices stay inside the picker", () => {
  const voices = [
    ...source.voice.voices,
    ...Array.from({ length: 32 }, (_, index) => ({
      id: `server-voice-${index}`,
      label: `Server choice ${index + 1}`,
    })),
  ];
  const f = setup({
    active: true,
    source: { ...source, voice: { ...source.voice, voices } },
  });
  assert.equal(f.find("Voice"), undefined);
  assert.equal(f.voicePicker(), undefined);
  f.openPreferences();
  assert.equal(f.button("Voice").props.value, "Cedar");
  assert.equal(f.voicePicker(), undefined);
  assert.equal(f.all().filter((row) => row.type === "SettingsRow").length, 1);
  for (const row of voices) {
    assert.equal(f.find(`${row.label} voice`), undefined);
    assert.equal(f.find(row.label), undefined);
  }
  f.button("Voice").props.onPress();
  const picker = f.voicePicker();
  assert.ok(picker);
  assert.equal(picker.props.label, "Voice");
  assert.equal(picker.props.title, "Choose voice");
  assert.equal(picker.props.description, "Changes apply to your next call.");
  assert.deepEqual(
    JSON.parse(JSON.stringify(picker.props.options)),
    voices.map((row) => ({ value: row.id, label: row.label })),
  );
  assert.equal(picker.props.options.length, 34);
  assert.deepEqual(JSON.parse(JSON.stringify(picker.props.value)), ["cedar"]);
  assert.deepEqual(JSON.parse(picker.props.scopeKey), [
    7,
    thread,
    JSON.stringify([source.service_id, null, "platform", source.model]),
  ]);
  assert.equal(f.savedPreferences.length, 0);
  assert.equal(f.voice.connected, true);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("browsing and cancelling the voice picker never saves or changes the current call", async () => {
  const f = setup({ active: true });
  f.openPreferences();
  f.button("Voice").props.onPress();
  const picker = f.voicePicker();
  assert.ok(picker);
  await f.harness.settle();
  assert.equal(f.savedPreferences.length, 0);
  assert.deepEqual(f.events, []);
  picker.props.onClose();
  f.harness.flush();
  assert.equal(f.voicePicker(), undefined);
  assert.equal(f.button("Voice").props.value, "Cedar");
  assert.deepEqual(f.events, []);
  assert.equal(f.voice.connected, true);
  f.harness.unmount();
});

test("an open voice picker keeps its presentation, choices and selection stable during query refreshes", () => {
  const f = setup({ active: true });
  f.openPreferences();
  f.button("Voice").props.onPress();
  const picker = f.voicePicker()!;
  assert.equal(picker.props.presentationStyle, "overFullScreen");
  const openingOptions = structuredClone(picker.props.options);
  const openingValue = structuredClone(picker.props.value);
  f.query.options.data = {
    options: [
      {
        ...source,
        voice: {
          ...source.voice,
          voices: [{ id: "marin", label: "Updated Marin" }],
        },
      },
    ],
  };
  f.query.settings.data = { voice: { ...saved, voice: "marin" } };
  f.voice.snapshot = {
    ...snapshot,
    tasks: [],
    captions: [
      {
        id: "live",
        speaker: "user",
        text: "Still talking",
        sealed: false,
        complete: false,
      },
    ],
  };
  f.harness.render();
  assert.deepEqual(
    JSON.parse(JSON.stringify(f.voicePicker()!.props.options)),
    openingOptions,
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(f.voicePicker()!.props.value)),
    openingValue,
  );
  assert.equal(f.voicePicker()!.key, picker.key);
  assert.equal(f.voice.connected, true);
  assert.equal(f.savedPreferences.length, 0);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("the voice picker stays open with Apply disabled through a failed refetch or temporary offline state", () => {
  const f = setup({ active: true });
  f.openPreferences();
  f.button("Voice").props.onPress();
  const opening = f.voicePicker()!;
  f.query.options.isSuccess = false;
  f.query.options.error = new Error("temporary network failure");
  f.harness.render();
  assert.ok(f.voicePicker(), "cached source identity keeps the picker mounted");
  assert.equal(f.voicePicker()!.props.disabled, true);
  assert.equal(f.voicePicker()!.props.keepOpenWhenDisabled, true);
  assert.equal(f.voicePicker()!.props.options, opening.props.options);
  f.query.options.isSuccess = true;
  f.query.options.error = null;
  f.setOnline(false);
  assert.ok(f.voicePicker());
  assert.equal(f.voicePicker()!.props.disabled, true);
  f.setOnline(true);
  assert.equal(f.voicePicker()!.props.disabled, false);
  assert.equal(f.savedPreferences.length, 0);
  f.harness.unmount();
});

test("a real provider error remains visible while the failed call is closing", () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    error: "Voice audio could not be verified.",
  });
  assert.ok(f.notices().includes("Voice audio could not be verified."));
  assert.match(f.text(), /previous call is finishing/);
  f.harness.unmount();
});

test("verified empty discovery does not dismiss an actionable billing failure", async () => {
  const f = setup({ ...creditFailure, conflictUnknown: true });
  f.voice.conflictUnknown = false;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.deepEqual(f.events, []);
  assert.ok(f.button("Review billing on Web"));
  assert.ok(f.button("Try again"));
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("applying a voice persists once and preserves the selected source and live call", async () => {
  const f = setup({ active: true });
  f.openPreferences();
  f.button("Voice").props.onPress();
  const picker = f.voicePicker();
  assert.ok(picker);
  picker.props.onChange(["marin"]);
  picker.props.onChange(["marin"]);
  picker.props.onClose();
  await f.harness.settle();
  assert.equal(f.voicePicker(), undefined);
  assert.deepEqual(JSON.parse(JSON.stringify(f.savedPreferences)), [
    { ...saved, voice: "marin" },
  ]);
  assert.equal(f.button("Voice").props.value, "Marin");
  assert.equal(f.events.filter(([event]) => event === "save").length, 1);
  assert.equal(
    f.events.some(([event]) =>
      ["start", "end", "mode"].includes(String(event)),
    ),
    false,
  );
  assert.equal(f.voice.connected, true);
  assert.equal(f.voice.snapshot?.session.id, sessionId);
  f.harness.unmount();
});

test("failed voice saves retain the previous selection and show a retryable error", async () => {
  const f = setup({ active: true });
  f.failSave();
  f.openPreferences();
  f.button("Voice").props.onPress();
  const picker = f.voicePicker();
  assert.ok(picker);
  picker.props.onChange(["marin"]);
  picker.props.onClose();
  await f.harness.settle();
  assert.equal(f.savedPreferences.length, 0);
  assert.equal(f.button("Voice").props.value, "Cedar");
  assert.match(f.notices().join(" "), /could not be saved/);
  f.button("Voice").props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(f.voicePicker()!.props.value)), [
    "cedar",
  ]);
  assert.equal(
    f.events.some(([event]) =>
      ["start", "end", "mode"].includes(String(event)),
    ),
    false,
  );
  assert.equal(f.voice.connected, true);
  f.harness.unmount();
});

test("opening the voice picker releases held input without ending the call", () => {
  const f = setup({
    active: true,
    saved: { ...saved, input_mode: "push_to_talk" },
  });
  f.openPreferences();
  f.voice.holding = true;
  f.voice.inputEnabled = true;
  f.button("Voice").props.onPress();
  assert.ok(f.voicePicker());
  assert.deepEqual(f.events, [["hold", false]]);
  assert.equal(f.voice.holding, false);
  assert.equal(f.voice.inputEnabled, false);
  assert.equal(f.voice.connected, true);
  f.harness.unmount();
});

test("dismissed or unfocused voice pickers ignore delayed Apply callbacks", async () => {
  for (const dismiss of ["picker", "preferences", "focus"] as const) {
    const f = setup({ active: true });
    f.openPreferences();
    f.button("Voice").props.onPress();
    const picker = f.voicePicker();
    assert.ok(picker);
    if (dismiss === "picker") picker.props.onClose();
    if (dismiss === "preferences") {
      f.button("Close voice preferences").props.onPress();
      f.harness.render();
    }
    if (dismiss === "focus") f.setFocused(false);
    f.harness.flush();
    picker.props.onChange(["marin"]);
    await f.harness.settle();
    assert.equal(f.savedPreferences.length, 0, dismiss);
    assert.equal(f.voicePicker(), undefined, dismiss);
    assert.equal(f.voice.connected, true, dismiss);
    f.harness.unmount();
  }
});

test("voice picker Apply is fenced to its account epoch, chat, and voice source", async () => {
  for (const change of ["epoch", "chat", "source"] as const) {
    const f = setup({ active: true });
    f.openPreferences();
    f.button("Voice").props.onPress();
    const picker = f.voicePicker();
    assert.ok(picker);
    if (change === "epoch") f.environment.epoch++;
    if (change === "chat") f.props.thread = `nyxa-${"b".repeat(32)}`;
    if (change === "source") {
      const own = { ...source, connection_id: "another-connection" };
      f.query.options.data = { options: [own] };
      f.query.settings.data = {
        voice: { ...saved, connection_id: own.connection_id },
      };
    }
    f.harness.render();
    picker.props.onChange(["marin"]);
    await f.harness.settle();
    assert.equal(f.savedPreferences.length, 0, change);
    assert.equal(f.voicePicker(), undefined, change);
    f.button("Voice").props.onPress();
    assert.notEqual(f.voicePicker()!.props.scopeKey, picker.props.scopeKey);
    f.harness.unmount();
  }
});

test("preference-save failures stay actionable and never imply the next call changed", async () => {
  const f = setup({ active: true });
  f.failSave();
  f.openPreferences();
  f.button("Voice").props.onPress();
  f.voicePicker()!.props.onChange(["marin"]);
  await f.harness.settle();
  assert.match(f.notices().join(" "), /could not be saved/);
  assert.equal(f.button("Voice").props.value, "Cedar");
  assert.equal(f.savedPreferences.length, 0);
  assert.equal(
    f.events.some(([event]) => event === "mode" || event === "end"),
    false,
  );
  f.harness.unmount();
});

test("End silences input and retains its media owner until bounded cleanup finishes, then returns to text without terminal proof", async () => {
  const pending = deferred<boolean>();
  const f = setup({ active: true });
  const originalSnapshot = structuredClone(f.voice.snapshot);
  const mediaOwner = f.all().find((row) => row.type === "VoiceMediaBridge")!;
  f.setEnd(pending.promise);
  f.voice.holding = true;
  f.button("End voice and close").props.onPress();
  f.harness.flush();
  assert.deepEqual(f.events, [["hold", false], ["end"]]);
  assert.equal(f.voice.inputEnabled, false);
  assert.match(f.text(), /Ending/);
  // The microphone cannot be reopened while End drains the call.
  assert.equal(f.button("Mute microphone").props.disabled, true);
  assert.equal(f.find("Try again"), undefined);
  assert.equal(f.find("Check call status"), undefined);
  assert.doesNotMatch(
    f.text(),
    /previous call is finishing|continue with text/i,
  );
  assert.equal(f.button("Return to chat").props.disabled, true);
  f.button("Return to chat").props.onPress();
  assert.equal(
    f.all().find((row) => row.type === "VoiceMediaBridge")?.props.media,
    mediaOwner.props.media,
  );
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  pending.resolve(false);
  await f.harness.settle();
  assert.deepEqual(f.events, [["hold", false], ["end"], ["close"]]);
  assert.deepEqual(f.voice.snapshot, originalSnapshot);
  assert.equal(f.voice.confirmedEnded, false);
  assert.equal(f.find("Request end again"), undefined);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("an End that drains after the call disconnects never shows the pre-call welcome", async () => {
  const pending = deferred<boolean>();
  const f = setup({ active: true });
  f.setEnd(pending.promise);
  f.button("End voice and close").props.onPress();
  // The engine disconnects before the bounded drain settles.
  f.voice.connected = false;
  f.harness.flush();
  assert.match(f.text(), /Ending/);
  assert.doesNotMatch(f.text(), /Hey, what can I do for you today/);
  assert.equal(f.find("Retry voice options"), undefined);
  pending.resolve(true);
  await f.harness.settle();
  assert.deepEqual(f.events.at(-1), ["close"]);
  f.harness.unmount();
});

test("a verified terminal End also returns to the text composer once", async () => {
  const f = setup({ active: true });
  f.setEnd(Promise.resolve(true));
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  f.harness.unmount();
});

test("a failed remote End returns to text after bounded cleanup without claiming success", async () => {
  const pending = deferred<boolean>();
  const f = setup({ active: true });
  f.setEnd(pending.promise);
  f.button("End voice and close").props.onPress();
  f.harness.flush();
  assert.deepEqual(f.events, [["end"]]);
  pending.reject(new Error("private transport details"));
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  assert.equal(f.voice.snapshot?.session.state, "active");
  assert.equal(f.voice.confirmedEnded, false);
  assert.doesNotMatch(f.text(), /private transport/);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("a late terminal snapshot after returning to text cannot close the surface twice", async () => {
  const f = setup({ active: true });
  f.setEnd(Promise.resolve(false));
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  f.voice.connected = false;
  f.voice.snapshot = {
    ...snapshot,
    session: { ...snapshot.session, state: "closed", end_requested: true },
  };
  f.harness.render();
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("returning after End does not fabricate a terminal snapshot before later empty discovery", async () => {
  const f = setup({ active: true });
  f.setEnd(Promise.resolve(false));
  f.button("End voice and close").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  f.voice.connected = false;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.equal(f.voice.snapshot?.session.state, "active");
  assert.deepEqual(f.events, [["end"], ["close"]]);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("verified empty discovery closes disconnected controls once and preserves the truthful partial snapshot", async () => {
  const retained: VoiceSnapshot = {
    ...snapshot,
    captions: [
      {
        id: "partial-caption",
        speaker: "assistant",
        text: "A retained partial reply",
        sealed: false,
        complete: false,
      },
    ],
  };
  const original = structuredClone(retained);
  const f = setup({ snapshot: retained });
  f.voice.checkingConflict = true;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.deepEqual(f.events, []);
  f.voice.checkingConflict = false;
  f.harness.render();
  await f.harness.settle();
  f.harness.render();
  await f.harness.settle();
  assert.deepEqual(f.events, [["close"]]);
  assert.deepEqual(f.voice.snapshot, original);
  assert.deepEqual(f.snapshots.at(-1), original);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("a confirmed terminal call in another conversation closes recovery without a local snapshot or replacement call", async () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: `nyxa-${"b".repeat(32)}`,
  });
  assert.match(f.text(), /Finishing call/);
  assert.equal(f.voice.snapshot, null);
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  f.harness.render();
  await f.harness.settle();
  assert.deepEqual(f.events, [["close"]]);
  assert.equal(f.voice.snapshot, null);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("empty discovery cannot close a connected, starting, unknown, or conflicting call", async () => {
  for (const guard of [
    "connected",
    "starting",
    "unknown",
    "conflict",
  ] as const) {
    const f = setup({ snapshot });
    f.voice.confirmedEnded = true;
    if (guard === "connected") f.voice.connected = true;
    if (guard === "starting") f.voice.starting = true;
    if (guard === "unknown") f.voice.conflictUnknown = true;
    if (guard === "conflict")
      f.voice.conflict = { ...snapshot.session, id: "another-session" };
    f.harness.render();
    await f.harness.settle();
    assert.deepEqual(f.events, [], guard);
    assert.equal(f.starts().length, 0, guard);
    f.harness.unmount();
  }
});

test("a disconnected nonterminal snapshot cannot expose a new start while recovery catches up", () => {
  const f = setup({ snapshot, error: "Voice connection lost" });
  assert.equal(f.find("Try again"), undefined);
  assert.ok(f.button("End voice and close"));
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("rapid End taps issue one request and a late completion cannot close another surface", async () => {
  const pending = deferred<boolean>();
  const f = setup({ active: true });
  f.setEnd(pending.promise);
  const end = f.button("End voice and close");
  end.props.onPress();
  end.props.onPress();
  assert.deepEqual(f.events, [["end"]]);
  f.harness.unmount();
  pending.resolve(true);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(f.events, [["end"]]);
});

test("checking unknown status is read-only and never masquerades as Ending", async () => {
  const pending = deferred<boolean>();
  const f = setup({ conflictUnknown: true });
  f.setCheck(pending.promise);
  assert.equal(f.find("End voice and close"), undefined);
  assert.equal(f.button("Return to chat").props.disabled, false);
  f.button("Check call status").props.onPress();
  f.harness.flush();
  assert.match(f.text(), /Checking call status/);
  assert.doesNotMatch(f.text(), /Ending/);
  assert.deepEqual(f.events, [["checkConflict"]]);
  pending.resolve(false);
  await f.harness.settle();
  assert.equal(
    f.events.some(([event]) => event === "close" || event === "end"),
    false,
  );
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("unknown or checking recovery can return to text without an End or a replacement call", async () => {
  for (const state of ["unknown", "checking"] as const) {
    const f = setup({
      conflictUnknown: state === "unknown",
      checkingConflict: state === "checking",
    });
    assert.equal(f.button("Return to chat").props.disabled, false, state);
    f.button("Return to chat").props.onPress();
    await f.harness.settle();
    assert.deepEqual(f.events, [["close"]], state);
    assert.equal(f.voice.conflictUnknown, state === "unknown", state);
    assert.equal(f.voice.checkingConflict, state === "checking", state);
    assert.equal(f.voice.snapshot, null, state);
    f.harness.unmount();
  }
});

test("returning during a status read leaves the other chat's call untouched and ignores late completion", async () => {
  const pending = deferred<boolean>();
  const f = setup({
    conflict: snapshot.session,
    conflictThread: `nyxa-${"b".repeat(32)}`,
  });
  const originalConflict = f.voice.conflict;
  f.setCheck(pending.promise);
  f.button("Check call status").props.onPress();
  f.harness.flush();
  assert.match(f.text(), /Checking call status/);
  assert.equal(f.button("Return to chat").props.disabled, false);
  f.button("Return to chat").props.onPress();
  assert.deepEqual(f.events, [["checkConflict"], ["close"]]);
  assert.equal(f.voice.conflict, originalConflict);
  f.harness.unmount();
  pending.resolve(true);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(f.events, [["checkConflict"], ["close"]]);
});

test("a pending previous End can be checked or left without repeating End or starting another call", async () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
  });
  const originalConflict = f.voice.conflict;
  assert.match(f.text(), /Finishing call/);
  assert.equal(f.starts().length, 0);
  assert.equal(f.find("End existing call"), undefined);
  assert.equal(f.find("Request end again"), undefined);
  await f.button("Check call status").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["checkConflict"]]);
  f.button("Return to chat").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["checkConflict"], ["close"]]);
  assert.equal(f.voice.conflict, originalConflict);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("another chat's existing call is never taken over or replaced implicitly", async () => {
  const f = setup({
    conflict: snapshot.session,
    conflictThread: `nyxa-${"b".repeat(32)}`,
  });
  assert.match(f.text(), /Call already active/);
  assert.match(f.text(), /open in another chat/);
  assert.equal(f.starts().length, 0);
  await f.button("End existing call").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["endConflict"], ["close"]]);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("ending an existing call hides recovery actions until cleanup returns to text once", async () => {
  const pending = deferred<boolean>();
  const f = setup({
    conflict: snapshot.session,
    conflictThread: `nyxa-${"b".repeat(32)}`,
  });
  const originalConflict = f.voice.conflict;
  f.setEndConflict(pending.promise);
  assert.ok(f.button("Check call status"));
  assert.equal(f.button("Return to chat").props.disabled, false);
  f.button("End existing call").props.onPress();
  f.harness.flush();
  assert.match(f.text(), /Ending/);
  assert.equal(f.find("Check call status"), undefined);
  assert.doesNotMatch(
    f.text(),
    /previous call is finishing|continue with text/i,
  );
  assert.equal(f.button("Return to chat").props.disabled, true);
  f.button("Return to chat").props.onPress();
  assert.deepEqual(f.events, [["endConflict"]]);
  pending.resolve(false);
  await f.harness.settle();
  assert.deepEqual(f.events, [["endConflict"], ["close"]]);
  assert.equal(f.voice.conflict, originalConflict);
  assert.equal(f.voice.confirmedEnded, false);
  assert.equal(f.starts().length, 0);
  f.harness.render();
  assert.deepEqual(f.events, [["endConflict"], ["close"]]);
  f.harness.unmount();
});

test("returning from another chat's active-call conflict does not End that call", async () => {
  const f = setup({
    conflict: snapshot.session,
    conflictThread: `nyxa-${"b".repeat(32)}`,
  });
  const originalConflict = f.voice.conflict;
  assert.ok(f.button("End existing call"));
  f.button("Return to chat").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["close"]]);
  assert.equal(f.voice.conflict, originalConflict);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("leaving or losing network during recovery never ends an unrelated call", async () => {
  for (const change of ["offline", "blur"] as const) {
    const f = setup({
      conflict: snapshot.session,
      conflictThread: `nyxa-${"b".repeat(32)}`,
    });
    if (change === "offline") f.setOnline(false);
    else f.setFocused(false);
    await f.harness.settle();
    assert.deepEqual(f.events, [], change);
    assert.equal(f.starts().length, 0, change);
    f.harness.unmount();
  }
});

test("clearing a detected conflict does not consume the same entry as a new call", () => {
  const f = setup({ conflict: snapshot.session });
  assert.equal(f.starts().length, 0);
  f.voice.conflict = null;
  f.voice.conflictUnknown = false;
  f.harness.render();
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("PTT uses press, release and cancellation while respecting server input gates", () => {
  const f = setup({
    active: true,
    saved: { ...saved, input_mode: "push_to_talk" },
    inputEnabled: false,
  });
  const hold = f.button("Hold to talk");
  hold.props.onPressIn();
  hold.props.onPressOut();
  hold.props.onResponderTerminate();
  assert.deepEqual(f.events, [
    ["hold", true],
    ["hold", false],
    ["hold", false],
  ]);
  hold.props.onAccessibilityAction({ nativeEvent: { actionName: "activate" } });
  assert.equal(f.voice.holding, true);
  f.voice.snapshot = {
    ...snapshot,
    session: { ...snapshot.session, input_muted: true },
  };
  f.harness.render();
  assert.equal(f.button("Hold to talk").props.disabled, true);
  f.harness.unmount();
});

test("ordinary snapshots keep the media owner and a new attempt remounts only its engine", () => {
  const f = setup({ active: true });
  const bridge = () => f.all().find((row) => row.type === "VoiceMediaBridge")!;
  const original = bridge();
  f.voice.snapshot = {
    ...snapshot,
    captions: [
      {
        id: "segment",
        speaker: "user",
        text: "你好",
        sealed: true,
        complete: true,
      },
    ],
  };
  f.harness.render();
  assert.equal(bridge().key, original.key);
  assert.equal(bridge().props.media, original.props.media);
  f.voice.mediaAttempt = 2;
  f.harness.render();
  assert.notEqual(bridge().key, original.key);
  f.harness.unmount();
});

test("offline and focus loss finish local cleanup and return to text without claiming remote termination", async () => {
  for (const change of ["offline", "blur"] as const) {
    const f = setup({ active: true });
    f.setEnd(Promise.resolve(false));
    if (change === "offline") f.setOnline(false);
    else f.setFocused(false);
    await f.harness.settle();
    assert.deepEqual(f.events, [["end"], ["close"]]);
    assert.equal(f.voice.snapshot?.session.state, "active");
    assert.equal(f.voice.confirmedEnded, false);
    f.harness.unmount();
  }
});

test("re-entering this chat during a requested disconnection starts exactly one new call after terminal proof", async () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: thread,
  });
  f.setStart(() => {
    f.voice.starting = true;
  });
  assert.match(f.text(), /Disconnecting previous call/);
  assert.match(f.text(), /new call will start once disconnection is confirmed/);
  assert.equal(f.starts().length, 0);
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.voice.snapshot = {
    ...snapshot,
    session: {
      ...snapshot.session,
      state: "closed",
      end_requested: true,
      end_reason: "user_ended",
    },
  };
  f.harness.render();
  await f.harness.settle();
  assert.match(f.text(), /Starting new call/);
  assert.equal(f.starts().length, 1);
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  f.harness.render();
  await f.harness.settle();
  assert.equal(f.starts().length, 1);
  f.voice.starting = false;
  f.voice.connected = true;
  f.voice.confirmedEnded = false;
  f.voice.snapshot = {
    ...snapshot,
    session: { ...snapshot.session, id: "new-session" },
  };
  f.harness.render();
  assert.doesNotMatch(
    f.text(),
    /Disconnecting previous call|Starting new call/,
  );
  f.harness.unmount();
});

test("replacing another chat's call requires the explicit action and waits for confirmed disconnection", async () => {
  const other = `nyxa-${"b".repeat(32)}`;
  const pending = deferred<boolean>();
  const f = setup({ conflict: snapshot.session, conflictThread: other });
  f.setStart(() => {
    f.voice.starting = true;
  });
  f.setEndConflict(pending.promise);
  assert.equal(f.starts().length, 0);
  assert.doesNotMatch(f.text(), /Disconnecting previous call/);
  f.button("End previous call and start here").props.onPress();
  f.harness.flush();
  assert.match(f.text(), /Disconnecting previous call/);
  assert.deepEqual(f.events, [["endConflict"]]);
  f.voice.conflict = {
    ...snapshot.session,
    state: "closing",
    end_requested: true,
  };
  pending.resolve(false);
  await f.harness.settle();
  assert.equal(f.starts().length, 0);
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.equal(f.starts().length, 1);
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  f.harness.unmount();
});

test("a queued restart is cancelled by background, focus, network, source, voice, return-to-text, or a newly discovered call", async () => {
  for (const change of [
    "background",
    "focus",
    "network",
    "source",
    "voice",
    "return",
    "replacement",
  ] as const) {
    const f = setup({
      conflict: { ...snapshot.session, state: "closing", end_requested: true },
      conflictThread: thread,
    });
    assert.match(f.text(), /Disconnecting previous call/, change);
    if (change === "background") f.appState("background");
    if (change === "focus") f.setFocused(false);
    if (change === "network") f.setOnline(false);
    if (change === "source") {
      f.query.options.data = {
        options: [{ ...source, connection_id: "another-source" }],
      };
      f.harness.render();
    }
    if (change === "voice") {
      f.query.settings.data!.voice = { ...saved, voice: "marin" };
      f.harness.render();
    }
    if (change === "return") f.button("Return to chat").props.onPress();
    if (change === "replacement") {
      f.voice.conflict = { ...snapshot.session, id: "new-call" };
      f.harness.render();
    }
    f.voice.conflict = null;
    f.voice.confirmedEnded = true;
    f.setFocused(true);
    f.setOnline(true);
    f.harness.render();
    await f.harness.settle();
    assert.equal(f.starts().length, 0, change);
    f.harness.unmount();
  }
});

test("provider, billing, idle, time-limit, and unknown terminal reasons cannot restart a queued call", async () => {
  for (const failure of [
    "provider",
    "billing",
    "idle",
    "time_limit",
    null,
  ] as const) {
    const f = setup({
      conflict: { ...snapshot.session, state: "closing", end_requested: true },
      conflictThread: thread,
    });
    f.voice.conflict = null;
    f.voice.confirmedEnded = true;
    if (failure === "provider") f.voice.error = "Provider could not connect";
    else if (failure === "billing")
      f.voice.recovery = {
        kind: "credits",
        message: "Add credits",
      } as VoiceStartRecovery;
    else
      f.voice.snapshot = {
        ...snapshot,
        session: { ...snapshot.session, state: "closed", end_reason: failure },
      };
    f.harness.render();
    await f.harness.settle();
    assert.equal(f.starts().length, 0, String(failure));
    assert.equal(
      f.events.some(([event]) => event === "close"),
      false,
    );
    if (failure !== "provider" && failure !== "billing")
      assert.match(f.notices().join(" "), /previous call ended/i);
    f.harness.unmount();
  }
});

test("a queued restart expires without making a new call or restarting after a later confirmation", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: thread,
  });
  context.mock.timers.tick(30_000);
  f.harness.flush();
  assert.match(f.text(), /still disconnecting.*try again shortly/i);
  assert.doesNotMatch(f.text(), /new call will start once/);
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("a retained closing call waits for late settings before arming its one restart", async () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: thread,
    settingsPending: true,
  });
  assert.doesNotMatch(f.text(), /new call will start once/);
  f.query.settings.data = { voice: saved };
  f.query.settings.isSuccess = true;
  f.query.settings.isPending = false;
  f.harness.render();
  assert.match(f.text(), /Disconnecting previous call/);
  f.setStart(() => {
    f.voice.starting = true;
  });
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.equal(f.starts().length, 1);
  f.harness.unmount();
});

test("a requested restart keeps reading status within its deadline after ordinary automatic checks finish", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: thread,
  });
  for (let i = 0; i < 8; i++) {
    f.runIntervals();
    await f.harness.settle();
  }
  assert.equal(
    f.events.filter(
      ([event, manual]) => event === "checkConflict" && manual === false,
    ).length,
    8,
  );
  assert.equal(f.starts().length, 0);
  context.mock.timers.tick(30_000);
  f.harness.flush();
  f.runIntervals();
  assert.equal(
    f.events.filter(([event]) => event === "checkConflict").length,
    8,
  );
  f.harness.unmount();
});

test("a restart waiting for local teardown stays visible until the new call becomes ready", async () => {
  const waiting = deferred<void>();
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: thread,
  });
  f.setStart(() => waiting.promise);
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.match(f.text(), /Starting new call/);
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  f.voice.starting = true;
  waiting.resolve();
  f.harness.render();
  await f.harness.settle();
  assert.match(f.text(), /Starting new call/);
  assert.equal(f.starts().length, 1);
  f.harness.unmount();
});

test("a restart rejected by the hook's final guards shows a retry message instead of waiting indefinitely", async () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
    conflictThread: thread,
  });
  f.setStart(() => false);
  f.voice.conflict = null;
  f.voice.confirmedEnded = true;
  f.harness.render();
  await f.harness.settle();
  assert.match(f.notices().join(" "), /new call did not start/i);
  assert.doesNotMatch(f.text(), /Starting new call/);
  assert.equal(f.starts().length, 1);
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  f.harness.unmount();
});

for (const viewport of [
  { width: 375, height: 812, fontScale: 1 },
  { width: 834, height: 1194, fontScale: 1 },
  { width: 375, height: 812, fontScale: 2.5 },
]) {
  test(`voice panel fits its content through startup, listening and live captions at ${viewport.width}pt/${viewport.fontScale}`, () => {
    const f = setup({ viewport });
    const height = () =>
      f.all().find((node) => node.props.testID === "voice-expanded-panel")!
        .props.style.height;
    const measureBody = (height: number) => {
      f.all()
        .find((node) => node.props.testID === "voice-panel-content")!
        .props.onLayout({ nativeEvent: { layout: { height } } });
      f.harness.flush();
    };
    const initial = height();
    assert.equal(
      initial,
      244,
      "only header, equalizer, gaps and controls reserve space",
    );
    assert.doesNotMatch(
      f.text(),
      /Hey, what can I do for you today|Change in Settings/,
    );
    f.voice.starting = true;
    f.harness.render();
    assert.equal(height(), initial);
    f.voice.starting = false;
    f.voice.connected = true;
    f.voice.snapshot = structuredClone(snapshot);
    f.harness.render();
    assert.equal(height(), initial);
    f.voice.snapshot.captions = [
      {
        id: "live",
        speaker: "user",
        text: "A longer spoken request expands the voice surface only as needed",
        sealed: false,
        complete: false,
      },
    ];
    f.harness.render();
    measureBody(160);
    assert.equal(height(), initial + 94);
    f.voice.snapshot.captions = [];
    f.harness.render();
    measureBody(66);
    assert.equal(height(), initial);
    f.harness.unmount();
  });
}

for (const reducedMotion of [false, true]) {
  test(`voice content grows before scrolling, animates its expansion, and shrinks after dismissal with Reduce Motion ${reducedMotion}`, () => {
    const f = setup({ active: true, reducedMotion });
    const panel = () =>
      f.all().find((node) => node.props.testID === "voice-expanded-panel")!;
    const scroll = () =>
      f.all().find((node) => node.props.testID === "voice-panel-scroll")!;
    const content = () =>
      f.all().find((node) => node.props.testID === "voice-panel-content")!;
    const measure = (height: number) => {
      content().props.onLayout({ nativeEvent: { layout: { height } } });
      f.harness.flush();
    };
    assert.equal(panel().props.style.height, 244);
    assert.equal(scroll().props.scrollEnabled, false);
    f.layoutAnimations.length = 0;
    f.voice.error = "Voice connection lost";
    f.harness.render();
    measure(240);
    assert.equal(panel().props.style.height, 418);
    assert.equal(scroll().props.scrollEnabled, false);
    assert.equal(scroll().props.bounces, false);
    assert.equal(f.layoutAnimations.length, reducedMotion ? 0 : 1);
    if (!reducedMotion)
      assert.equal(
        (f.layoutAnimations[0] as { duration: number }).duration,
        220,
      );
    f.voice.error = undefined;
    f.harness.render();
    measure(66);
    assert.equal(panel().props.style.height, 244);
    assert.equal(scroll().props.scrollEnabled, false);
    assert.equal(f.layoutAnimations.length, reducedMotion ? 0 : 2);
    f.harness.unmount();
  });
}

for (const viewport of [
  { width: 375, height: 667, fontScale: 1 },
  { width: 402, height: 874, fontScale: 1 },
  { width: 402, height: 874, fontScale: 2.5 },
  { width: 320, height: 568, fontScale: 1 },
]) {
  test(`voice only scrolls after reaching 60% of the ${viewport.height}pt screen at text scale ${viewport.fontScale}`, () => {
    const f = setup({ active: true, viewport, preferencesOpen: true });
    const panel = () =>
      f.all().find((node) => node.props.testID === "voice-expanded-panel")!;
    const scroll = () =>
      f.all().find((node) => node.props.testID === "voice-panel-scroll")!;
    const measure = (id: string, height: number) => {
      f.all()
        .find((node) => node.props.testID === id)!
        .props.onLayout({
          nativeEvent: { layout: { height } },
        });
      f.harness.flush();
    };
    // Simulate native text wrapping, rather than guessing a fixed notice height.
    measure("voice-panel-header", 80);
    measure("voice-panel-controls", 86);
    const cap =
      Math.min(viewport.height * 0.6, viewport.height - 47 - 34 - 60 - 96) - 24;
    const bodyAtCap = cap - 80 - 86 - 20;
    measure("voice-panel-content", Math.floor(bodyAtCap));
    assert.equal(scroll().props.scrollEnabled, false);
    measure("voice-panel-content", Math.ceil(bodyAtCap) + 100);
    assert.equal(panel().props.style.height, cap);
    assert.equal(scroll().props.scrollEnabled, true);
    assert.equal(scroll().props.showsVerticalScrollIndicator, true);
    const descendants = elements(scroll());
    for (const id of ["voice-panel-header", "voice-panel-controls"])
      assert.equal(
        descendants.some((node) => node.props.testID === id),
        false,
      );
    assert.equal(
      f.all().filter((node) => node.type === "ScrollView").length,
      1,
      "preferences, captions and tasks share one scroll area",
    );
    measure("voice-panel-content", Math.max(0, Math.floor(bodyAtCap) - 20));
    assert.equal(scroll().props.scrollEnabled, false);
    f.harness.unmount();
  });
}
