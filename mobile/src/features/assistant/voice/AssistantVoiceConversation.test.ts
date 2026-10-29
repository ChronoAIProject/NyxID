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
  } = {},
) {
  const events: unknown[][] = [];
  let focused = true;
  let online = true;
  let browserFails = false;
  const environment = { origin: "https://nyx.example", epoch: 7 };
  const voice = {
    media: { id: 1 },
    mediaAttempt: 1,
    starting: false,
    connected: input.active ?? false,
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
  };
  const harness = hookHarness<Element>({
    "../../../components/workspace/useNativeSheetDismiss": {
      useNativeSheetDismiss: (onClose: () => void) => ({
        modalProps: { visible: true, animationType: "slide" },
        close: () => onClose(),
      }),
    },
    "../../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => false,
    },
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
        [
          "ActivityIndicator",
          "Modal",
          "Pressable",
          "ScrollView",
          "Switch",
          "Text",
          "View",
        ].map((name) => [name, name]),
      ),
      StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 1 },
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
    "lucide-react-native": Object.fromEntries(
      [
        "ChevronDown",
        "ChevronLeft",
        "Mic",
        "MicOff",
        "Minus",
        "PhoneOff",
        "Volume2",
        "VolumeX",
        "X",
      ].map((name) => [name, name]),
    ),
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
        colors: { primaryTone: { bg: "pink", border: "pink" } },
      }),
    },
    "../../../theme/designTokens": { fonts: {} },
    "./useNativeVoice": { useNativeVoice: () => voice },
    "./VoiceMediaBridge": { VoiceMediaBridge: "VoiceMediaBridge" },
  });
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
    events,
    voice,
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
  assert.equal(f.button("Start voice").props.disabled, false);
  await f.button("Start voice").props.onPress();
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
  await f.button("Start voice").props.onPress();
  assert.equal(
    (f.events[0]?.[1] as VoicePreferences).input_mode,
    "push_to_talk",
  );
  assert.equal(f.events[0]?.[2], "xai_realtime");
  f.button("Microphone: Hold to talk").props.onPress();
  assert.equal(
    f
      .all()
      .some((row) => row.props.accessibilityLabel?.startsWith("Automatic")),
    false,
  );
  f.harness.unmount();
});

test("unavailable defaults cannot start and display clear guidance", () => {
  const f = setup({ source: { ...source, available: false } });
  assert.equal(f.button("Start voice").props.disabled, true);
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
  await f.button("Start voice").props.onPress();
  assert.equal(
    (f.events.find((event) => event[0] === "start")?.[1] as VoicePreferences)
      ?.model,
    source.model,
  );
  f.harness.unmount();
});

test("minimizing releases push-to-talk before hiding the sheet and keeps the media owner mounted", () => {
  const f = setup({ active: true });
  f.voice.holding = true;
  const media = f.media();
  f.button("Minimize voice call").props.onPress();
  assert.deepEqual(f.events, [["hold", false]]);
  assert.equal(f.modal().visible, false);
  assert.equal(f.media().key, media.key);
  assert.equal(f.media().props.media, media.props.media);
  f.button("Restore voice call").props.onPress();
  assert.equal(f.modal().visible, true);
  assert.equal(
    f.events.some(([event]) => event === "end"),
    false,
  );
  f.harness.unmount();
});

test("reviewing an approval releases held audio and leaves both the call and task running", () => {
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
          title: "Review this action",
        },
      ],
    },
  });
  f.voice.holding = true;
  f.button("Review in chat").props.onPress();
  assert.deepEqual(f.events, [
    ["hold", false],
    ["review", "ack-1"],
  ]);
  assert.equal(f.modal().visible, false);
  assert.equal(
    f.events.some(([event]) => event === "end" || event === "stopTask"),
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

test("native Back returns from a picker before dismissing the voice surface", () => {
  const f = setup();
  f.button("Voice: Cedar").props.onPress();
  f.button("Back to voice call");
  f.modal().onRequestClose();
  assert.equal(
    f
      .all()
      .some((row) => row.props.accessibilityLabel === "Back to voice call"),
    false,
  );
  assert.deepEqual(f.events, []);
  f.modal().onRequestClose();
  assert.deepEqual(f.events, [["end"], ["close"]]);
  f.harness.unmount();
});

test("closing the surface ends the call while Stop task calls only the cancellation API", () => {
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
      ],
    },
  });
  f.button("Stop task").props.onPress();
  assert.deepEqual(f.events, [["stopTask", serviceId]]);
  f.button("End voice and close").props.onPress();
  assert.deepEqual(f.events, [["stopTask", serviceId], ["end"], ["close"]]);
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

test("loss of connectivity ends local voice and disables starting until the app reconnects", () => {
  const f = setup();
  f.setOnline(false);
  assert.equal(f.button("Start voice").props.disabled, true);
  assert.deepEqual(f.events, [["end"]]);
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
