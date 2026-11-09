import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "../assistantHookTestHarness";
import type { VoiceMediaEvent } from "./AssistantVoiceClient";
import type { VoiceStartRecovery } from "./voiceStartError";
import type {
  VoiceOption,
  VoicePreferences,
  VoiceSession,
  VoiceSnapshot,
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
const creditFailure = {
  error: "Insufficient voice credits (billing_reservation · code 11300)",
  recovery: {
    kind: "credits" as const,
    message:
      "There aren’t enough available credits to reserve this voice call. Review your balance and applicable benefits in Billing, or choose another eligible voice source.",
  },
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
    inputEnabled?: boolean;
  } = {},
) {
  const events: unknown[][] = [];
  const savedPreferences: VoicePreferences[] = [];
  const snapshots: (VoiceSnapshot | undefined)[] = [];
  const captures: { listening: boolean; inputActive: boolean }[] = [];
  const listeners = new Set<(event: VoiceMediaEvent) => void>();
  let online = true;
  let focused = true;
  let browserFails = false;
  let saveFails = false;
  let endResult: Promise<boolean> = Promise.resolve(true);
  let checkResult: Promise<boolean> = Promise.resolve(false);
  let endConflictResult: Promise<boolean> = Promise.resolve(false);
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
  const media = {
    id: 1,
    subscribe: (listener: (event: VoiceMediaEvent) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
  const levels = {
    input: { value: 0 },
    output: { value: 0 },
    speaking: false,
    media: undefined as unknown,
  };
  const voice = {
    media: media as typeof media | null,
    mediaAttempt: 1,
    starting: false,
    connected: input.active ?? false,
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
    checkingConflict: false,
    start: async (preferences: VoicePreferences, protocol: string) => {
      events.push(["start", preferences, protocol]);
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
    checkConflict: () => {
      events.push(["checkConflict"]);
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
    setInputMode: (value: string) => {
      events.push(["mode", value]);
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
    preferencesOpen: false,
    onClose: () => {
      events.push(["close"]);
    },
    onUpdate: () => {},
    onReview: (id?: string) => {
      events.push(["review", id]);
    },
    onSnapshot: (value: VoiceSnapshot | undefined) => snapshots.push(value),
    onCaptureState: (value: { listening: boolean; inputActive: boolean }) =>
      captures.push({ ...value }),
    onPreferencesClose: () => {
      props.preferencesOpen = false;
    },
  };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
    },
    "react-native": {
      ...Object.fromEntries(
        ["ActivityIndicator", "Pressable", "ScrollView", "Text", "View"].map(
          (name) => [name, name],
        ),
      ),
      StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 1 },
    },
    "react-native-reanimated": {
      default: { View: "WaveBar" },
      View: "WaveBar",
      useAnimatedStyle: (factory: () => unknown) => factory(),
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "@tanstack/react-query": {
      useQuery: ({ queryKey }: { queryKey: unknown[] }) =>
        queryKey.includes("voice-options") ? query.options : query.settings,
    },
    "expo-web-browser": {
      openBrowserAsync: async (url: string) => {
        events.push(["billing", url]);
        if (browserFails) throw new Error("private browser error");
      },
    },
    "lucide-react-native": Object.fromEntries(
      ["Ellipsis", "Mic", "MicOff", "PhoneOff", "X"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => input.reducedMotion ?? false,
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
          primaryTone: { bg: "pink" },
          warningTone: { bg: "amber", text: "amber-text" },
          dangerTone: { bg: "red" },
        },
      }),
    },
    "../../../theme/designTokens": { fonts: {} },
    "../../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Card: "Card",
      Label: "Text",
      Notice: "Notice",
    },
    "../ChatComposer": { ChatComposerFrame: "ComposerFrame" },
    "./VoiceMediaBridge": { VoiceMediaBridge: "VoiceMediaBridge" },
    "./useNativeVoice": { useNativeVoice: () => voice },
    "./useVoiceLevels": {
      useVoiceLevels: (value: unknown) => {
        levels.media = value;
        return levels;
      },
    },
  });
  const component = harness.load(
    fileURLToPath(new URL("./AssistantVoiceConversation.tsx", import.meta.url)),
  ).AssistantVoiceConversation as (props: unknown) => Element;
  harness.mount(() => expand(component(props)));
  const all = () => elements(harness.flush());
  const find = (label: string) =>
    all().find(
      (row) =>
        row.props.accessibilityLabel === label ||
        (row.type === "ActionButton" && row.props.label === label),
    );
  const button = (label: string) => {
    const result = find(label);
    assert.ok(result, `Expected voice control: ${label}`);
    return result;
  };
  return {
    harness,
    events,
    voice,
    levels,
    query,
    props,
    savedPreferences,
    snapshots,
    captures,
    all,
    find,
    button,
    text: () =>
      all()
        .flatMap((row) => row.props.children.flat(Infinity))
        .filter((child) => typeof child === "string")
        .join(" "),
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
    failBrowser: () => {
      browserFails = true;
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
    setOnline: (value: boolean) => {
      online = value;
      harness.render();
    },
    setFocused: (value: boolean) => {
      focused = value;
      harness.render();
    },
  };
}

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
  assert.doesNotMatch(
    f.text(),
    /Hey, what can I do|Tap to start|Ready to talk/,
  );
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
  assert.match(f.text(), /Grok supports Hold to talk/);
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

test("normal voice is a narrow control strip with no welcome, captions, mode picker or internal transcript", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      captions: [
        {
          id: "segment",
          speaker: "user",
          text: "do not duplicate this text",
          sealed: true,
          complete: true,
        },
      ],
    },
  });
  assert.match(f.text(), /Listening/);
  assert.doesNotMatch(
    f.text(),
    /do not duplicate|Hey, what can I do|Hands-free|Settings/,
  );
  assert.equal(
    f.all().some((row) => row.type === "ScrollView"),
    false,
  );
  assert.equal(
    f.all().some((row) => row.type === "Modal"),
    false,
  );
  assert.ok(f.button("Mute microphone"));
  assert.ok(f.button("End voice"));
  assert.equal(
    f.find("Voice preferences"),
    undefined,
    "top-right header owns preferences",
  );
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
  assert.deepEqual(f.captures.at(-1), { listening: false, inputActive: false });
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
  assert.match(f.text(), /Microphone paused/);
  assert.doesNotMatch(f.text(), /Listening|Assistant is speaking/);
  f.voice.inputEnabled = true;
  f.harness.render();
  assert.match(f.text(), /Listening/);
  f.levels.speaking = true;
  f.harness.render();
  assert.match(f.text(), /Assistant is speaking/);
  f.voice.speakerMuted = true;
  f.harness.render();
  assert.doesNotMatch(f.text(), /Assistant is speaking/);
  f.harness.unmount();
});

test("the capture placeholder follows real input levels and clears immediately on mute", () => {
  const f = setup({ active: true });
  assert.deepEqual(f.captures.at(-1), { listening: true, inputActive: false });
  f.emit({ type: "level", input: 0.5, output: 0 });
  assert.deepEqual(f.captures.at(-1), { listening: true, inputActive: true });
  f.voice.inputEnabled = false;
  f.voice.snapshot = {
    ...snapshot,
    session: { ...snapshot.session, muted: true },
  };
  f.harness.render();
  assert.deepEqual(f.captures.at(-1), { listening: false, inputActive: false });
  assert.match(f.text(), /Microphone off/);
  assert.ok(f.button("Unmute microphone"));
  f.harness.unmount();
});

test("mute is an input action and speaker mute stays inside preferences", () => {
  const f = setup({ active: true });
  f.button("Mute microphone").props.onPress();
  assert.equal(f.find("Mute speaker"), undefined);
  f.openPreferences();
  f.button("Mute speaker").props.onPress();
  assert.deepEqual(f.events, [
    ["mute", true],
    ["speaker", true],
  ]);
  assert.ok(f.button("No sound? Resume audio"));
  f.harness.unmount();
});

test("voice and mode changes are saved for next call without changing this call", async () => {
  const f = setup({ active: true });
  f.openPreferences();
  assert.match(f.text(), /Changes apply to your next call/);
  await f.button("Hold to talk mode").props.onPress();
  await f.harness.settle();
  assert.deepEqual(
    f.savedPreferences.map((row) => row.input_mode),
    ["push_to_talk"],
  );
  assert.equal(
    f.events.some(([event]) => event === "mode"),
    false,
  );
  assert.equal(f.find("Hold to talk"), undefined);
  await f.button("Marin voice").props.onPress();
  await f.harness.settle();
  assert.equal(f.savedPreferences.at(-1)?.voice, "marin");
  assert.equal(f.savedPreferences.at(-1)?.service_id, saved.service_id);
  assert.equal(f.savedPreferences.at(-1)?.input_mode, "push_to_talk");
  f.harness.unmount();
});

test("preference-save failures stay actionable and never imply the live call changed", async () => {
  const f = setup({ active: true });
  f.failSave();
  f.openPreferences();
  await f.button("Hold to talk mode").props.onPress();
  await f.harness.settle();
  assert.match(f.notices().join(" "), /could not be saved/);
  assert.equal(
    f.button("Hands-free mode").props.accessibilityState.selected,
    true,
  );
  assert.equal(
    f.events.some(([event]) => event === "mode"),
    false,
  );
  f.harness.unmount();
});

test("pricing is fetched from the selected option and stays folded by default", () => {
  const f = setup({ active: true });
  assert.doesNotMatch(f.text(), /tariff|credits per/);
  f.openPreferences();
  assert.doesNotMatch(f.text(), /tariff unavailable/);
  f.button("View voice pricing").props.onPress();
  assert.match(f.text(), /Voice tariff unavailable/);
  f.button("Hide voice pricing").props.onPress();
  assert.doesNotMatch(f.text(), /Voice tariff unavailable/);
  f.harness.unmount();
});

test("End closes local input immediately but keeps pending confirmation visible and blocks restart", async () => {
  const pending = deferred<boolean>();
  const f = setup({ active: true });
  f.setEnd(pending.promise);
  f.voice.holding = true;
  f.button("End voice").props.onPress();
  f.harness.flush();
  assert.deepEqual(f.events, [["hold", false], ["end"]]);
  assert.equal(f.voice.inputEnabled, false);
  assert.match(f.text(), /Ending/);
  assert.equal(f.find("Mute microphone"), undefined);
  assert.equal(f.find("Try again"), undefined);
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  pending.resolve(false);
  await f.harness.settle();
  assert.match(f.text(), /Confirming call ended/);
  assert.ok(f.button("Check call status"));
  assert.ok(f.button("Request end again"));
  assert.equal(
    f.events.some(([event]) => event === "close"),
    false,
  );
  f.harness.unmount();
});

test("only a verified terminal End returns to the text composer", async () => {
  const f = setup({ active: true });
  f.setEnd(Promise.resolve(true));
  f.button("End voice").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  f.harness.unmount();
});

test("a disconnected nonterminal snapshot cannot expose a new start while recovery catches up", () => {
  const f = setup({ snapshot, error: "Voice connection lost" });
  assert.equal(f.find("Try again"), undefined);
  assert.ok(f.button("End voice"));
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
});

test("rapid End taps issue one request and a late completion cannot close another surface", async () => {
  const pending = deferred<boolean>();
  const f = setup({ active: true });
  f.setEnd(pending.promise);
  const end = f.button("End voice");
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
  assert.equal(f.button("End voice").props.disabled, true);
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

test("pending End can be checked or explicitly retried and never automatically starts another call", async () => {
  const f = setup({
    conflict: { ...snapshot.session, state: "closing", end_requested: true },
  });
  assert.match(f.text(), /Confirming call ended/);
  assert.equal(f.starts().length, 0);
  assert.equal(f.find("End existing call"), undefined);
  await f.button("Check call status").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["checkConflict"]]);
  f.setEndConflict(Promise.resolve(false));
  await f.button("Request end again").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events, [["checkConflict"], ["endConflict"]]);
  assert.equal(f.starts().length, 0);
  f.setCheck(Promise.resolve(true));
  await f.button("Check call status").props.onPress();
  await f.harness.settle();
  assert.deepEqual(f.events.at(-1), ["close"]);
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
  assert.deepEqual(f.events, [["endConflict"]]);
  assert.equal(f.starts().length, 0);
  f.harness.unmount();
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
      ],
    },
  });
  assert.match(f.text(), /Needs your OK/);
  assert.equal(f.find("Stop task"), undefined);
  f.button("Show agent tasks").props.onPress();
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

test("offline and focus loss silence an existing call without automatically returning to a restartable composer", async () => {
  for (const change of ["offline", "blur"] as const) {
    const f = setup({ active: true });
    f.setEnd(Promise.resolve(false));
    if (change === "offline") f.setOnline(false);
    else f.setFocused(false);
    await f.harness.settle();
    assert.deepEqual(f.events, [["end"]]);
    assert.match(f.text(), /Confirming call ended/);
    assert.equal(
      f.events.some(([event]) => event === "close"),
      false,
    );
    f.harness.unmount();
  }
});

test("reduced motion keeps the small waveform still while retaining live status", () => {
  const f = setup({ active: true, reducedMotion: true });
  const bars = f.all().filter((row) => row.type === "WaveBar");
  assert.equal(bars.length, 5);
  assert.ok(
    bars.every((row) => row.props.style.at(-1).transform[0].scaleY === 1),
  );
  assert.match(f.text(), /Listening/);
  f.harness.unmount();
});
