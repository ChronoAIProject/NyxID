import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistantHookTestHarness";
import type { VoiceStartRecovery } from "./voiceStartError";
import type {
  VoiceOption,
  VoicePreferences,
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
    saved?: VoicePreferences;
    snapshot?: VoiceSnapshot;
    error?: string;
    recovery?: VoiceStartRecovery;
    reducedMotion?: boolean;
    actions?: unknown;
    actionCount?: number;
  } = {},
) {
  const events: unknown[][] = [];
  let focused = true;
  let online = true;
  const keyboard = new Map<string, (event: any) => void>();
  let browserFails = false;
  const environment = {
    origin: "https://nyx.example",
    epoch: 7,
    api: {
      savePreferences: async (preferences: VoicePreferences) => {
        events.push(["save", preferences.input_mode]);
      },
    },
  };
  const levels = {
    input: { value: 0 },
    output: { value: 0 },
    speaking: false,
    media: undefined as unknown,
  };
  const voice = {
    media: { id: 1 },
    mediaAttempt: 1,
    starting: false,
    connected: input.active ?? false,
    connectedAt: input.active ? Date.now() : undefined,
    holding: false,
    speakerMuted: false,
    snapshot: input.snapshot ?? (input.active ? snapshot : null),
    error: input.error,
    recovery: input.recovery,
    conflict: null,
    endingConflict: false,
    start: async (preferences: VoicePreferences, protocol: string) => {
      events.push(["start", preferences, protocol]);
    },
    end: async () => {
      events.push(["end"]);
    },
    endConflict: async () => {
      events.push(["endConflict"]);
    },
    hold: (value: boolean) => {
      events.push(["hold", value]);
      voice.holding = value;
    },
    mute: async (value: boolean) => {
      events.push(["mute", value]);
    },
    muteSpeaker: (value: boolean) => {
      events.push(["speaker", value]);
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
      data: { options: [input.source ?? source] },
      isSuccess: true,
      isPending: false,
      error: null as Error | null,
      refetch: async () => {},
    },
    settings: {
      data: { voice: input.saved ?? saved },
      isSuccess: true,
      isPending: false,
      error: null as Error | null,
      refetch: async () => {},
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
    actions: input.actions,
    actionCount: input.actionCount,
  };
  const harness = hookHarness<Element>({
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
      LayoutAnimation: { configureNext() {}, Presets: { easeInEaseOut: {} } },
      StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 1 },
      useWindowDimensions: () => ({ width: 390, height: 844 }),
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
        "Ellipsis",
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
    "./VoiceComposerAura": { VoiceComposerAura: "VoiceAura" },
    "./VoiceMediaBridge": { VoiceMediaBridge: "VoiceMediaBridge" },
  });
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
  const one = (predicate: (row: Element) => boolean) => {
    const result = all().find(predicate);
    assert.ok(result, "Expected voice control");
    return result;
  };
  return {
    harness,
    props,
    keyboard,
    events,
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
    button: (label: string) =>
      one(
        (row) =>
          row.props.accessibilityLabel === label ||
          (row.type === "ActionButton" && row.props.label === label),
      ),
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
  f.button("Continue with text").props.onPress();
  assert.deepEqual(f.events.slice(1), [["end"], ["close"]]);
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
  assert.equal(f.button("Connecting voice").props.disabled, true);
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

test("closing voice releases push-to-talk and ends the call", () => {
  const f = setup({ active: true });
  f.voice.holding = true;
  f.button("End voice and close").props.onPress();
  assert.deepEqual(f.events, [["hold", false], ["end"], ["close"]]);
  f.harness.unmount();
});

test("voice leaves approval details and task receipts in the chat", () => {
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
          title: "Private task receipt",
        },
      ],
    },
  });
  assert.doesNotMatch(
    f.text(),
    /Private task receipt|Agent tasks|Needs your OK/,
  );
  assert.equal(
    f.all().some((row) => row.props.label === "Stop task"),
    false,
  );
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
  f.voice.media = { id: 2 };
  f.voice.mediaAttempt = 2;
  f.harness.render();
  assert.notEqual(f.media().key, first.key);
  assert.notEqual(f.media().props.media, first.props.media);
  f.harness.unmount();
});

test("closing voice before a call returns to the message composer", () => {
  const f = setup();
  f.button("Close voice").props.onPress();
  assert.deepEqual(
    f.events.map((e) => e[0]),
    ["start", "end", "close"],
  );
  f.harness.unmount();
});

test("closing voice ends the call and leaves accepted tasks running", () => {
  const f = setup({ active: true });
  f.button("End voice and close").props.onPress();
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
  phone.button("Call options").props.onPress();
  phone.harness.render();
  assert.ok(
    phone.button("No sound? Resume audio"),
    "helpers live in the ⋯ menu",
  );
  phone.harness.unmount();
});

test("loss of connectivity ends local voice and disables starting until the app reconnects", () => {
  const f = setup();
  f.setOnline(false);
  assert.equal(f.button("Try again").props.disabled, true);
  assert.deepEqual(
    f.events.map((e) => e[0]),
    ["start", "end"],
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

test("round call controls mute the microphone and speaker and End closes the call", () => {
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

test("before a call the screen names the saved voice and mode, set in Settings", () => {
  const f = setup();
  assert.match(f.text(), /Cedar voice/);
  assert.match(f.text(), /Hands-free/);
  assert.match(f.text(), /Settings › Voice/);
  assert.match(f.text(), /Connecting/);
  assert.equal(f.aura().phase, "resting");
  f.harness.unmount();
});

test("only unsealed live captions appear in the voice composer", () => {
  const f = setup({
    active: true,
    snapshot: {
      ...snapshot,
      captions: ["one", "two", "three"].map((text, index) => ({
        id: `caption-${index}`,
        speaker: index % 2 ? ("assistant" as const) : ("user" as const),
        text,
        sealed: index < 2,
        complete: index < 2,
      })),
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

test("voice is a composer with the animated bubble on top, not a page over the chat", () => {
  const idle = setup();
  assert.equal(
    idle.all().some((row) => row.type === "Modal"),
    false,
  );
  const frame = idle.all().find((row) => row.type === "ComposerFrame")!;
  assert.ok(frame, "it takes the message composer's place");
  assert.ok(
    idle.all().some((row) => row.type === "VoiceAura"),
    "the aura around the composer",
  );
  assert.match(idle.text(), /我的助手/);
  assert.equal(idle.button("Connecting voice").props.disabled, true);
  assert.equal(idle.events[0]?.[0], "start");
  idle.harness.unmount();
  // Hands-free: the microphone in the middle, End on the right.
  const f = setup({ active: true });
  f.button("Mute microphone").props.onPress();
  f.button("End voice and close").props.onPress();
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
    ["start", "save"],
  );
  assert.deepEqual(idle.events.at(-1), ["save", "push_to_talk"]);
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

test("minimizing preserves media and session; tapping or typing expands and text uses the supplied chat send", () => {
  const f = setup({ active: true });
  const media = f.media();
  const textEvents: any[] = [];
  Object.assign(f.props, {
    draft: "",
    onChangeText: (text: string) => textEvents.push(text),
    onSendText: () => textEvents.push("send"),
    canSendText: true,
  });
  f.props.minimized = true;
  f.harness.render();
  assert.equal(f.media().key, media.key);
  assert.equal(f.media().props.media, media.props.media);
  assert.equal(f.events.length, 0);
  assert.equal(
    f.all().some((n) => n.props.accessibilityLabel === "Listening mode"),
    false,
  );
  f.button("Expand voice composer").props.onPress();
  const input = f.all().find((n) => n.type === "TextInput")!;
  input.props.onFocus();
  input.props.onChangeText("Keep working");
  assert.deepEqual(textEvents, ["Keep working"]);
  assert.deepEqual(
    f.events.map((e) => e[0]),
    ["expand", "expand", "expand"],
  );
  input.props.onSubmitEditing();
  assert.deepEqual(textEvents, ["Keep working", "send"]);
  f.props.minimized = false;
  f.harness.render();
  assert.equal(f.media().key, media.key);
  assert.equal(
    f.events.some((e) => e[0] === "end" || e[0] === "start"),
    false,
  );
  f.keyboard.get("keyboardDidShow")!({ endCoordinates: { screenY: 330 } });
  f.harness.flush();
  assert.ok(
    f
      .all()
      .some((n) => n.type === "ScrollView" && n.props.style?.maxHeight < 140),
  );
  f.harness.unmount();
});

test("paid tariff and saved voice remain visible while auto-start is connecting", () => {
  const f = setup({
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
  f.voice.starting = true;
  f.harness.render();
  assert.match(f.text(), /Cedar voice/);
  assert.match(f.text(), /0.025 credits per second/);
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Voice tariff"));
  assert.ok(f.all().some((n) => n.props.accessibilityLabel === "Call time -"));
  assert.equal(f.events.filter((e) => e[0] === "start").length, 1);
  f.harness.unmount();
});
