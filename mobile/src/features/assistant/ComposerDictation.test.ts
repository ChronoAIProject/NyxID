import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import type { ComposerDictationState } from "./useComposerDictation";

type Element = { type: string; props: Record<string, any> };
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
function setup({ introSeen = true, granted = false } = {}) {
  const events: string[] = [];
  const menus: { onChange: (value: string) => void; options: unknown[] }[] = [];
  const saved: { language?: string } = {};
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
  };
  const h = hookHarness<Element>({
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => false,
    },
    "./ChatComposer": { ChatComposerFrame: "ChatComposerFrame" },
    "../display/ChatLayoutContext": { useComposerHeight: () => 120 },
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
    "expo-linear-gradient": { LinearGradient: "LinearGradient" },
    "expo-speech-recognition": {
      ExpoSpeechRecognitionModule: {
        getSupportedLocales: async () => ({ locales: ["en-US", "fr-FR"] }),
        getPermissionsAsync: async () => ({ granted }),
      },
    },
    "../../components/settings/SettingsList": {
      usePopupMenu: (options: {
        onChange: (value: string) => void;
        options: unknown[];
      }) => {
        menus.push(options);
        return { anchor: { current: null }, open: () => {}, popup: null };
      },
    },
    "./voice/dictationLocale": {
      dictationLocale: (phone: string, list: readonly string[]) =>
        list.includes(phone) ? phone : list[0],
    },
    "./voice/dictationLanguage": {
      languageLabel: (locale: string) => `name:${locale}`,
      languageOptions: (list: readonly string[]) =>
        list.map((value) => ({ value, label: `name:${value}` })),
      savedDictationLanguage: async () => saved.language,
      saveDictationLanguage: async (value: string | undefined) => {
        saved.language = value;
        events.push(`language:${value ?? "auto"}`);
      },
    },
    "react-native": {
      useWindowDimensions: () => ({ width: 390, height: 844, fontScale: 1 }),
      ActivityIndicator: "ActivityIndicator",
      Keyboard: { dismiss: () => events.push("keyboard") },
      Modal: "Modal",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      Text: "Text",
      View: "View",
      StyleSheet: {
        create: (styles: unknown) => styles,
        hairlineWidth: 1,
        absoluteFill: {},
      },
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
  h.mock(
    "./DictationWaveform",
    h.load(fileURLToPath(new URL("./DictationWaveform.tsx", import.meta.url))),
  );
  const components = h.load(
    fileURLToPath(new URL("./ComposerDictation.tsx", import.meta.url)),
  ) as {
    ComposerDictationButton: (props: {
      dictation: ComposerDictationState;
      disabled?: boolean;
    }) => Element;
    ComposerDictationNotice: (props: {
      dictation: ComposerDictationState;
    }) => Element | null;
    ComposerDictationReview: (props: {
      dictation: ComposerDictationState;
      draft?: string;
    }) => Element | null;
  };
  /** The mic button is the whole control: no intro sheet or window. */
  const mic = () => components.ComposerDictationButton({ dictation });
  return {
    h,
    dictation,
    events,
    mic,
    storage,
    levelListeners,
    menus,
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
    render(x.ComposerDictationNotice({ dictation: x.dictation })) as Element,
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

test("listening stays in the composer: words above a streaming waveform, Discard and Finish", () => {
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
  assert.equal(
    tree.some((node) => node.type === "Modal"),
    false,
    "no sheet or full-screen page covers the chat",
  );
  const texts = tree.filter((node) => node.type === "Text");
  const interim = texts.find((node) =>
    [node.props.children].flat().includes("for two"),
  )!;
  assert.equal(interim.props.style.color, "#888888");
  const words = tree.find((node) => node.type === "ScrollView")!;
  assert.ok(
    tree.indexOf(words) <
      tree.findIndex(
        (node) =>
          node.props.importantForAccessibility === "no-hide-descendants",
      ),
    "the words sit above the waveform line",
  );
  const done = tree.find(
    (node) => node.props.accessibilityLabel === "Finish dictation",
  )!;
  assert.equal(done.props.disabled, false);
  assert.match(done.props.accessibilityHint, /Nothing is sent/);
  done.props.onPress();
  tree
    .find((node) => node.props.accessibilityLabel === "Discard dictation")!
    .props.onPress();
  assert.deepEqual(x.events, ["stop", "discard"]);
});

test("with nothing said yet the waveform takes the field's place; an existing draft stays before new words", () => {
  const x = setup();
  x.dictation.phase = "listening";
  x.dictation.active = true;
  x.dictation.recording = true;
  const empty = all(
    render(x.ComposerDictationReview({ dictation: x.dictation })) as Element,
  );
  assert.equal(
    empty.some((node) => node.type === "ScrollView"),
    true,
  );
  x.dictation.final = "and a window seat";
  x.dictation.text = "and a window seat";
  const tree = all(
    render(
      x.ComposerDictationReview({
        dictation: x.dictation,
        draft: "Book a table for two ",
      }),
    ) as Element,
  );
  const draft = tree.find(
    (node) =>
      node.type === "Text" &&
      [node.props.children].flat().includes("Book a table for two"),
  )!;
  assert.ok(draft, "the draft is shown without its trailing space");
  assert.deepEqual([draft.props.children].flat(), [
    "Book a table for two",
    " ",
  ]);
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
  assert.deepEqual(heights, [2, 2 + 0.5 * 20, 2 + 20]);
  x.h.unmount();
  assert.equal(x.levelListeners.size, 0, "unsubscribed on unmount");
});

test("the first tap dictates in the composer at once; no intro sheet or window opens", async () => {
  const x = setup({ introSeen: false });
  const button = x.mic();
  assert.equal(button.type, "Pressable");
  assert.match(
    button.props.accessibilityHint,
    /Nothing is sent until you tap Send/,
  );
  button.props.onPress();
  await tick();
  assert.deepEqual(x.events, ["keyboard", "start"]);
  const held = setup({ introSeen: false });
  held.mic().props.onLongPress();
  assert.deepEqual(
    held.events,
    ["keyboard", "start"],
    "push-to-talk starts at once",
  );
});

test("dictation grows within the typing height limit and keeps the meter between Discard and Finish", () => {
  const x = setup();
  x.dictation.active = true;
  for (const phase of [
    "permission",
    "connecting",
    "listening",
    "stopping",
  ] as const) {
    x.dictation.phase = phase;
    const tree = render(
      x.ComposerDictationReview({ dictation: x.dictation }),
    ) as Element;
    assert.equal(
      tree.props.style,
      undefined,
      "content controls the composer height",
    );
    const textArea = all(tree).find((node) => node.type === "ScrollView")!;
    assert.equal(textArea.props.style[0].flexGrow, 0);
    assert.deepEqual(
      { ...textArea.props.style[1] },
      { minHeight: 44, maxHeight: 120 },
    );
    const row = all(tree).find(
      (node) =>
        node.type === "View" &&
        [node.props.children]
          .flat()
          .some(
            (child: any) =>
              child?.props?.accessibilityLabel === "Discard dictation",
          ),
    )!;
    const children = [row.props.children].flat().filter(Boolean);
    assert.equal(children.length, 3);
    assert.equal(children[0].props.accessibilityLabel, "Discard dictation");
    assert.equal(children[2].props.accessibilityLabel, "Finish dictation");
    assert.ok(
      all(children[1]).some(
        (node) => node.type === "Text" || node.type === "ActivityIndicator",
      ),
    );
  }
  x.dictation.error = "Microphone unavailable";
  const review = render(
    x.ComposerDictationReview({ dictation: x.dictation }),
  ) as Element;
  assert.equal(
    JSON.stringify(review).includes("Microphone unavailable"),
    false,
  );
  assert.ok(x.ComposerDictationNotice({ dictation: x.dictation }));
});
