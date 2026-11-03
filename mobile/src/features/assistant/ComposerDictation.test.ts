import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import type { ComposerDictationState } from "./useComposerDictation";

type Element = { type: string; props: Record<string, any> };
function setup() {
  const events: string[] = [];
  const dictation: ComposerDictationState = {
    phase: "idle",
    text: "",
    error: undefined,
    permissionDenied: false,
    media: null,
    origin: undefined,
    openSettings: () => events.push("settings"),
    active: false,
    recording: false,
    start: async () => {
      events.push("start");
    },
    stop: () => {
      events.push("stop");
    },
    discard: () => {
      events.push("discard");
    },
    insert: () => {
      events.push("insert");
    },
  };
  const h = hookHarness<Element>({
    "react/jsx-runtime": Object.fromEntries(
      ["jsx", "jsxs"].map((key) => [
        key,
        (type: string, props: Record<string, any>) => ({ type, props }),
      ]),
    ),
    "./voice/VoiceMediaBridge": { VoiceMediaBridge: "VoiceMediaBridge" },
    "react-native": {
      ActivityIndicator: "ActivityIndicator",
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": { Mic: "Mic", Square: "Square" },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { composerBg: "#222222" } }),
    },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Label: "Label",
    },
    "../../components/workspace/ScrollEdgeFades": {
      OverflowScrollView: "OverflowScrollView",
    },
  });
  const components = h.load(
    fileURLToPath(new URL("./ComposerDictation.tsx", import.meta.url)),
  ) as {
    ComposerDictationButton: (props: {
      dictation: ComposerDictationState;
      disabled?: boolean;
    }) => Element;
    ComposerDictationReview: (props: {
      dictation: ComposerDictationState;
    }) => Element | null;
  };
  return { h, dictation, events, ...components };
}
function all(element: Element): Element[] {
  return [
    element,
    ...[element.props.children]
      .flat(Infinity)
      .flatMap((child: unknown) =>
        child && typeof child === "object" && "props" in child
          ? all(child as Element)
          : [],
      ),
  ];
}

test("dictation keeps a labelled 44pt microphone/Stop target with explicit pending states", async () => {
  const x = setup();
  let button = x.ComposerDictationButton({ dictation: x.dictation });
  assert.equal(button.props.accessibilityLabel, "Dictate message");
  assert.deepEqual(
    { ...button.props.style({ pressed: false })[0] },
    {
      width: 44,
      height: 44,
      alignItems: "center",
      justifyContent: "center",
    },
  );
  await button.props.onPress();
  assert.deepEqual(x.events, ["start"]);
  x.dictation.phase = "listening";
  x.dictation.recording = true;
  button = x.ComposerDictationButton({ dictation: x.dictation });
  assert.equal(button.props.accessibilityLabel, "Stop dictation");
  button.props.onPress();
  assert.equal(x.events.at(-1), "stop");
  for (const phase of ["permission", "connecting", "stopping"] as const) {
    x.dictation.phase = phase;
    button = x.ComposerDictationButton({ dictation: x.dictation });
    assert.equal(button.props.disabled, true);
    assert.equal(button.props.accessibilityState.busy, true);
  }
});

test("idle dictation adds no blank review space, while long review text is scrollable with fades", () => {
  const x = setup();
  assert.equal(x.ComposerDictationReview({ dictation: x.dictation }), null);
  x.dictation.phase = "review";
  x.dictation.active = true;
  x.dictation.text = "Long dictated message";
  const tree = all(x.ComposerDictationReview({ dictation: x.dictation })!);
  const scroller = tree.find((node) => node.type === "OverflowScrollView")!;
  assert.equal(scroller.props.fadeColor, "#222222");
  assert.equal(scroller.props.style.maxHeight, 120);
  const buttons = tree.filter((node) => node.type === "ActionButton");
  assert.deepEqual(
    buttons.map((node) => node.props.label),
    ["Insert text", "Discard dictation"],
  );
  buttons[0]!.props.onPress();
  buttons[1]!.props.onPress();
  assert.deepEqual(x.events, ["insert", "discard"]);
});

test("the first tap after a released long press starts dictation again", () => {
  const x = setup();
  const button = x.ComposerDictationButton({ dictation: x.dictation });
  button.props.onLongPress();
  button.props.onPressOut();
  assert.deepEqual(x.events, ["start", "stop"]);
  // Native Pressability does not call onPress after onLongPress. This is the
  // next distinct gesture, after capture/review has returned to idle.
  button.props.onPress();
  assert.deepEqual(x.events, ["start", "stop", "start"]);
});
