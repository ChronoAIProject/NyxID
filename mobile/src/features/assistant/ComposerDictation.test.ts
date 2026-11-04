import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import type { ComposerDictationState } from "./useComposerDictation";

type Element = { type: string; props: Record<string, any> };
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
function setup({ introSeen = true } = {}) {
  const events: string[] = [];
  const levelListeners = new Set<(level: number) => void>();
  const storage = new Map<string, string>(
    introSeen ? [["nyxid.dictation_intro_seen", "1"]] : [],
  );
  const dictation: ComposerDictationState = {
    phase: "idle",
    text: "",
    final: "",
    interim: "",
    error: undefined,
    permissionDenied: false,
    subscribeLevel: (listener) => {
      levelListeners.add(listener);
      return () => {
        levelListeners.delete(listener);
      };
    },
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
    "react/jsx-runtime": {
      ...Object.fromEntries(
        ["jsx", "jsxs"].map((key) => [
          key,
          (type: string, props: Record<string, any>) => ({ type, props }),
        ]),
      ),
      Fragment: "Fragment",
    },
    "@react-native-async-storage/async-storage": {
      __esModule: true,
      default: {
        getItem: async (key: string) => storage.get(key) ?? null,
        setItem: async (key: string, value: string) => {
          storage.set(key, value);
          events.push("intro-saved");
        },
      },
    },
    "react-native-safe-area-context": {
      useSafeAreaInsets: () => ({ top: 47, bottom: 34, left: 0, right: 0 }),
    },
    "react-native": {
      ActivityIndicator: "ActivityIndicator",
      Keyboard: { dismiss: () => events.push("keyboard") },
      Modal: "Modal",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      Text: "Text",
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": {
      Check: "Check",
      Globe: "Globe",
      ShieldCheck: "ShieldCheck",
      Smartphone: "Smartphone",
      Mic: "Mic",
      Square: "Square",
      X: "X",
    },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: {
          composerBg: "#222222",
          bg: "#111111",
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
  /** The mic button itself; the button renders beside its intro sheet. */
  const mic = () =>
    components
      .ComposerDictationButton({ dictation })
      .props.children.find((child: Element) => child.type === "Pressable");
  const intro = () =>
    components
      .ComposerDictationButton({ dictation })
      .props.children.find((child: Element) => child.type !== "Pressable");
  return {
    h,
    dictation,
    events,
    mic,
    intro,
    storage,
    levelListeners,
    ...components,
  };
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
  let button = x.mic();
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
  button.props.onPress();
  await tick();
  assert.deepEqual(x.events, ["keyboard", "start"]);
  x.dictation.phase = "listening";
  x.dictation.recording = true;
  button = x.mic();
  assert.equal(button.props.accessibilityLabel, "Stop dictation");
  button.props.onPress();
  assert.equal(x.events.at(-1), "stop");
  for (const phase of ["permission", "connecting", "stopping"] as const) {
    x.dictation.phase = phase;
    button = x.mic();
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

test("listening fills the screen with a live transcript, meter and Finish", () => {
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
  assert.equal(tree[0]!.type, "Modal");
  assert.equal(tree[0]!.props.presentationStyle, "fullScreen");
  const texts = tree.filter((node) => node.type === "Text");
  assert.ok(texts.some((node) => node.props.children === "Speech to Text"));
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

test("review keeps the full transcript scrollable and selectable, offering Insert instead of Finish", () => {
  const x = setup();
  x.dictation.phase = "review";
  x.dictation.active = true;
  x.dictation.final = "Long dictated message";
  x.dictation.text = "Long dictated message";
  const tree = all(
    render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
  );
  const scroller = tree.find((node) => node.type === "ScrollView")!;
  assert.equal(scroller.props.style.flex, 1);
  const transcript = all(scroller).find(
    (node) => node.type === "Text" && node.props.selectable !== undefined,
  )!;
  assert.equal(transcript.props.selectable, true);
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

test("the first tap after a released long press starts dictation again", async () => {
  const x = setup();
  const button = x.mic();
  button.props.onLongPress();
  button.props.onPressOut();
  assert.deepEqual(x.events, ["keyboard", "start", "stop"]);
  // Native Pressability does not call onPress after onLongPress. This is the
  // next distinct gesture, after capture/review has returned to idle.
  button.props.onPress();
  await tick();
  assert.deepEqual(x.events, [
    "keyboard",
    "start",
    "stop",
    "keyboard",
    "start",
  ]);
});

test("the first tap explains dictation once; Start begins it and a held press skips the intro", async () => {
  const x = setup({ introSeen: false });
  x.h.mount(() => x.ComposerDictationButton({ dictation: x.dictation }));
  const children = () => x.h.value().props.children as Element[];
  const mic = () => children().find((child) => child.type === "Pressable")!;
  const sheet = () =>
    render(children().find((child) => child.type !== "Pressable")) as Element;
  const find = (label: string) =>
    all(sheet()).find((node) => node.props.accessibilityLabel === label)!;
  assert.equal(sheet().props.visible, false);
  assert.equal(sheet().props.transparent, true, "a compact sheet over the chat");
  mic().props.onPress();
  await x.h.settle();
  assert.deepEqual(x.events, ["keyboard"], "nothing records before Start");
  assert.equal(sheet().props.visible, true);
  find("Close").props.onPress();
  x.h.flush();
  assert.equal(sheet().props.visible, false);
  assert.deepEqual(x.events, ["keyboard"], "closing starts nothing");
  mic().props.onPress();
  await x.h.settle();
  find("Start dictation").props.onPress();
  await x.h.settle();
  assert.deepEqual([...x.events].sort(), [
    "intro-saved",
    "keyboard",
    "keyboard",
    "start",
  ]);
  assert.equal(x.storage.get("nyxid.dictation_intro_seen"), "1");
  const held = setup({ introSeen: false });
  held.mic().props.onLongPress();
  assert.deepEqual(
    held.events,
    ["keyboard", "start"],
    "push-to-talk starts at once",
  );
});

test("the meter draws native volume levels as bars and stops listening when capture ends", () => {
  const x = setup();
  x.dictation.phase = "listening";
  x.dictation.active = true;
  x.dictation.recording = true;
  const tree = () =>
    all(
      render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
    );
  x.h.mount(() => tree() as unknown as Element);
  const wave = () =>
    (x.h.value() as unknown as Element[]).find(
      (node) => node.props.importantForAccessibility === "no-hide-descendants",
    )!;
  wave().props.onLayout({ nativeEvent: { layout: { width: 15 } } });
  x.h.flush();
  assert.equal(x.levelListeners.size, 1, "subscribed while capturing");
  for (const listener of x.levelListeners) {
    listener(0.5);
    listener(1);
  }
  x.h.flush();
  const heights = all(wave())
    .filter((node) => node.type === "View" && node !== wave())
    .map((node) => node.props.style[1].height);
  assert.deepEqual(heights, [3, 3 + 0.5 * 33, 3 + 33]);
  x.h.unmount();
  assert.equal(x.levelListeners.size, 0, "unsubscribed on unmount");
});

test("copy says recognition happens on the iPhone in its language, not on a server", () => {
  const x = setup({ introSeen: false });
  const intro = all(render(x.intro()) as Element);
  const words = intro
    .filter((node) => node.type === "Text")
    .flatMap((node) => [node.props.children].flat())
    .join(" ");
  assert.match(words, /recognized on your iPhone when available/);
  assert.match(words, /Uses your iPhone's language/);
  assert.doesNotMatch(words, /server|automatically/i);
  x.dictation.phase = "listening";
  x.dictation.active = true;
  const panel = all(
    render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
  );
  assert.ok(
    panel.some(
      (node) =>
        node.type === "Text" &&
        node.props.children === "Language: your iPhone's language",
    ),
  );
});
