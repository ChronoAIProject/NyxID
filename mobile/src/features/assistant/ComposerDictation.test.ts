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
    final: "",
    interim: "",
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
      Keyboard: { dismiss: () => events.push("keyboard") },
      Pressable: "Pressable",
      Text: "Text",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": {
      Check: "Check",
      Mic: "Mic",
      Square: "Square",
      X: "X",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: {
          composerBg: "#222222",
          textPrimary: "#ffffff",
          textTertiary: "#888888",
        },
      }),
    },
    "../../theme/designTokens": { fonts: {} },
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
// Expands local function components and Pressable render-prop children so the
// assertions see the native tree a person would.
function render(node: unknown): unknown {
  if (Array.isArray(node)) return node.map(render);
  if (!node || typeof node !== "object" || !("props" in node)) return node;
  const element = node as { type: any; props: Record<string, any> };
  if (typeof element.type === "function")
    return render(element.type(element.props));
  const children =
    typeof element.props.children === "function"
      ? element.props.children({ pressed: false })
      : element.props.children;
  return {
    type: element.type,
    props: { ...element.props, children: render(children) },
  };
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
  assert.deepEqual(x.events, ["keyboard", "start"]);
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

test("idle dictation adds no blank space; an idle error is a compact dismissible notice", () => {
  const x = setup();
  assert.equal(x.ComposerDictationReview({ dictation: x.dictation }), null);
  x.dictation.error = "Allow microphone access in Settings to dictate.";
  x.dictation.permissionDenied = true;
  const tree = all(
    render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
  );
  const buttons = tree.filter((node) => node.type === "ActionButton");
  assert.deepEqual(
    buttons.map((node) => [node.props.label, node.props.compact]),
    [
      ["Open Settings", true],
      ["Dismiss", true],
    ],
  );
  assert.equal(
    buttons[1]!.props.accessibilityLabel,
    "Dismiss dictation message",
  );
  buttons[0]!.props.onPress();
  buttons[1]!.props.onPress();
  assert.deepEqual(x.events, ["settings", "discard"]);
});

test("listening takes over the composer with a live transcript, meter and Done", () => {
  const x = setup();
  x.dictation.phase = "listening";
  x.dictation.active = true;
  x.dictation.recording = true;
  x.dictation.final = "Book a table";
  x.dictation.interim = "for two";
  x.dictation.text = "Book a table for two";
  const tree = all(
    render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
  );
  const texts = tree.filter((node) => node.type === "Text");
  assert.ok(texts.some((node) => node.props.children === "Listening"));
  const interim = texts.find((node) =>
    [node.props.children].flat().includes("for two"),
  )!;
  assert.equal(interim.props.style.color, "#888888");
  assert.ok(
    tree.some(
      (node) => node.props.importantForAccessibility === "no-hide-descendants",
    ),
    "the level meter is hidden from assistive technology",
  );
  const done = tree.find(
    (node) => node.props.accessibilityLabel === "Finish dictation",
  )!;
  assert.equal(done.props.disabled, false);
  done.props.onPress();
  tree
    .find((node) => node.props.accessibilityLabel === "Discard dictation")!
    .props.onPress();
  assert.deepEqual(x.events, ["stop", "discard"]);
  assert.equal(
    tree.some((node) => node.props.accessibilityLabel === "Insert text"),
    false,
  );
});

test("review keeps long text scrollable with fades and offers Insert instead of Done", () => {
  const x = setup();
  x.dictation.phase = "review";
  x.dictation.active = true;
  x.dictation.final = "Long dictated message";
  x.dictation.text = "Long dictated message";
  const tree = all(
    render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
  );
  const scroller = tree.find((node) => node.type === "OverflowScrollView")!;
  assert.equal(scroller.props.fadeColor, "#222222");
  assert.equal(scroller.props.style.maxHeight, 168);
  assert.equal(
    tree.some((node) => node.props.accessibilityLabel === "Finish dictation"),
    false,
  );
  tree
    .find((node) => node.props.accessibilityLabel === "Insert text")!
    .props.onPress();
  tree
    .find((node) => node.props.accessibilityLabel === "Discard dictation")!
    .props.onPress();
  assert.deepEqual(x.events, ["insert", "discard"]);
});

test("pending phases keep Discard available while Done waits for capture", () => {
  const x = setup();
  x.dictation.active = true;
  for (const phase of ["permission", "connecting"] as const) {
    x.dictation.phase = phase;
    const tree = all(
      render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
    );
    const done = tree.find(
      (node) => node.props.accessibilityLabel === "Finish dictation",
    )!;
    assert.equal(done.props.disabled, true);
    assert.ok(
      tree.some(
        (node) => node.props.accessibilityLabel === "Discard dictation",
      ),
    );
  }
});

test("the first tap after a released long press starts dictation again", () => {
  const x = setup();
  const button = x.ComposerDictationButton({ dictation: x.dictation });
  button.props.onLongPress();
  button.props.onPressOut();
  assert.deepEqual(x.events, ["keyboard", "start", "stop"]);
  // Native Pressability does not call onPress after onLongPress. This is the
  // next distinct gesture, after capture/review has returned to idle.
  button.props.onPress();
  assert.deepEqual(x.events, [
    "keyboard",
    "start",
    "stop",
    "keyboard",
    "start",
  ]);
});
