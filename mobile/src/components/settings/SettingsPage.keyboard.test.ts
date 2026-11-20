import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";
import { spacing } from "../../theme/designTokens";

type Element = { type: unknown; props: Record<string, unknown> };
type Measure = (x: number, y: number, width: number, height: number) => void;
type Frame = { screenY: number; height: number };
type Geometry = {
  view: { y: number; height: number };
  keyboard: Frame;
  provider: { y: number; height: number };
  windowHeight: number;
};
const path = (file: string) => fileURLToPath(new URL(file, import.meta.url));
const nodes = (node: Element): Element[] => [
  node,
  ...[node.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? nodes(child as Element)
        : [],
    ),
];
function invoke(node: Element, name: string, ...args: unknown[]) {
  const callback = node.props[name];
  assert.equal(typeof callback, "function", name);
  return (callback as (...values: unknown[]) => unknown)(...args);
}

/** Exercise SettingsPage, its real scroller and the shared keyboard observer. */
function setup({
  resized = false,
  reduced = false,
  platform = "ios",
  geometry,
  avatar = false,
}: {
  resized?: boolean;
  reduced?: boolean;
  platform?: string;
  geometry?: Geometry;
  avatar?: boolean;
} = {}) {
  const listeners = new Map<
    string,
    (event: { endCoordinates: Frame }) => void
  >();
  const layouts = new Map<number, () => void>();
  let layoutId = 0;
  let appState = "active";
  let appChange: (state: string) => void = () => {};
  let metrics: Frame | undefined;
  const windowHeight = geometry?.windowHeight ?? 852;
  let viewportHeight = geometry?.view.height ?? (resized ? 433 : 699);
  let inputY = 650;
  let inputHeight = 120;
  let delayed = false;
  let offset = 0;
  const measurements: Measure[] = [];
  const scrolls: { y: number; animated: boolean }[] = [];
  const input = {
    measureInWindow: (callback: Measure) => {
      if (delayed) measurements.push(callback);
      else callback(20, inputY, 340, inputHeight);
    },
  };
  let focused: typeof input | null = null;
  const textInput = { State: { currentlyFocusedInput: () => focused } };
  const jsx = (type: unknown, props: Element["props"]): Element =>
    typeof type === "function"
      ? (type as (props: unknown) => Element)(props)
      : { type, props };
  const h = hookHarness<Element>(
    {
      "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
      "react-native": {
        View: "View",
        ScrollView: "ScrollView",
        TextInput: textInput,
        Platform: { OS: platform },
        StyleSheet: { create: (styles: unknown) => styles },
        useWindowDimensions: () => ({
          width: 393, height: windowHeight, fontScale: 1,
        }),
        AppState: {
          get currentState() {
            return appState;
          },
          addEventListener: (_: string, callback: typeof appChange) => {
            appChange = callback;
            return { remove() {} };
          },
        },
        Dimensions: {
          get: (name: string) => {
            assert.ok(name === "window" || name === "screen");
            return { height: windowHeight };
          },
          addEventListener: () => ({ remove() {} }),
        },
        Keyboard: {
          metrics: () => metrics,
          isVisible: () => !!metrics,
          scheduleLayoutAnimation() {},
          addListener: (
            name: string,
            callback: (event: { endCoordinates: Frame }) => void,
          ) => {
            listeners.set(name, callback);
            return { remove: () => listeners.delete(name) };
          },
        },
      },
      "../../app/WorkspaceHeader": { useWorkspaceSheet: () => null },
      "react-native-safe-area-context": {
        useSafeAreaInsets: () => ({ top: 0, bottom: 34 }),
        useSafeAreaFrame: () => geometry?.provider ?? { y: 0, height: 852 },
      },
      "expo-linear-gradient": { LinearGradient: "Gradient" },
      "lucide-react-native": {},
      "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
      "../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => reduced,
      },
      "../DynamicTypeText": { DynamicTypeText: "Text" },
      "../ScreenContainer": { ScreenContainer: "ScreenContainer" },
    },
    {
      requestAnimationFrame: (callback: () => void) => {
        layouts.set(++layoutId, callback);
        return layoutId;
      },
      cancelAnimationFrame: (id: number) => layouts.delete(id),
    },
  );
  h.mock(
    "./KeyboardViewport",
    h.load(path("../workspace/KeyboardViewport.tsx")),
  );
  h.mock(
    "./KeyboardScrollView",
    h.load(path("../workspace/KeyboardScrollView.tsx")),
  );
  h.mock(
    "../workspace/ScrollEdgeFades",
    h.load(path("../workspace/ScrollEdgeFades.tsx")),
  );
  const { SettingsPage, SettingsTextField } = h.load(
    path("./SettingsList.tsx"),
  ) as {
    SettingsPage: (props: unknown) => Element;
    SettingsTextField: (props: unknown) => Element;
  };
  const forwarded = { current: null as unknown };
  h.mount(() =>
    SettingsPage({
      scrollRef: forwarded,
      children: SettingsTextField({
        accessibilityLabel: avatar ? "Avatar" : "Add a memory",
        autoFocus: avatar,
        multiline: !avatar,
        value: "Draft",
      }),
    }),
  );
  const all = () => nodes(h.value());
  const scroll = () => all().find((node) => node.type === "ScrollView")!;
  const viewport = () =>
    all().find(
      (node) =>
        typeof node.props.onLayout === "function" &&
        Array.isArray(node.props.style),
    )!;
  const nativeScroll = {
    scrollTo: (position: { y: number; animated: boolean }) => {
      scrolls.push({ ...position });
      inputY -= position.y - offset;
      offset = position.y;
      invoke(scroll(), "onScroll", {
        nativeEvent: { contentOffset: { y: offset } },
      });
    },
  };
  invoke(scroll(), "ref", nativeScroll);
  const viewportRef = viewport().props.ref as {
    current: { measureInWindow: (callback: Measure) => void };
  };
  viewportRef.current = {
    measureInWindow: (callback) =>
      callback(0, geometry?.view.y ?? 119, 393, viewportHeight),
  };
  const flushLayout = () => {
    h.flush();
    const pending = [...layouts.values()];
    layouts.clear();
    for (const callback of pending) callback();
    h.flush();
  };
  return {
    h,
    all,
    scrolls,
    scroll,
    forwarded,
    nativeScroll,
    measurements,
    bottom: () =>
      (viewport().props.style as [unknown, { paddingBottom: number }])[1]
        .paddingBottom,
    flushLayout,
    focus: () => {
      focused = input;
      invoke(scroll(), "onFocus", {});
    },
    blur: () => {
      focused = null;
      invoke(scroll(), "onBlur", {});
    },
    show: () => {
      metrics = geometry?.keyboard ?? { screenY: 552, height: 300 };
      listeners.get(
        platform === "ios" ? "keyboardWillChangeFrame" : "keyboardDidShow",
      )!({ endCoordinates: metrics });
      flushLayout();
    },
    hide: () => {
      metrics = undefined;
      listeners.get("keyboardDidHide")!({
        endCoordinates: { screenY: windowHeight, height: 0 },
      });
      h.flush();
    },
    app: (state: string) => {
      appState = state;
      appChange(state);
      flushLayout();
    },
    resetField: (y = 650, height = 120) => {
      inputY = y;
      inputHeight = height;
      offset = 0;
      invoke(scroll(), "onScroll", {
        nativeEvent: { contentOffset: { y: 0 } },
      });
    },
    scrollFromNative: (y: number) => {
      inputY -= y - offset;
      offset = y;
      invoke(scroll(), "onScroll", { nativeEvent: { contentOffset: { y } } });
    },
    delayMeasurement: () => {
      delayed = true;
    },
    resize: (height: number) => {
      viewportHeight = height;
      invoke(viewport(), "onLayout");
      flushLayout();
    },
    contentSizeChange: () => {
      invoke(scroll(), "onContentSizeChange", 393, 1000);
      flushLayout();
    },
    fieldBottom: () => inputY + inputHeight,
  };
}

test("SettingsPage reserves only its measured keyboard overlap and scrolls the whole multiline field above it", () => {
  const env = setup();
  assert.equal(
    env.all().find((node) => node.props.accessibilityLabel === "Add a memory")
      ?.props.multiline,
    true,
  );
  env.focus();
  env.show();
  assert.equal(env.bottom(), 266);
  assert.deepEqual(env.scrolls, [
    { y: 650 + 120 - (552 - spacing.lg), animated: true },
  ]);
  assert.ok(env.fieldBottom() <= 552 - spacing.lg);
  env.resize(699);
  assert.equal(env.bottom(), 266, "inset never accumulates on layout");
  assert.equal(env.scrolls.length, 1);
  assert.equal(
    env.forwarded.current,
    env.nativeScroll,
    "existing caller refs still reach the native scroller",
  );
  assert.equal(env.scroll().props.automaticallyAdjustKeyboardInsets, false);
  env.hide();
  assert.equal(env.bottom(), 0);
  env.h.unmount();
});

test("an already resized SettingsPage sheet adds zero keyboard inset and still reveals its focused field", () => {
  const env = setup({ resized: true });
  env.focus();
  env.show();
  assert.equal(env.bottom(), 0);
  assert.ok(env.fieldBottom() <= 552 - spacing.lg);
  assert.equal(env.scrolls.length, 1);
  env.resize(433);
  assert.equal(env.bottom(), 0);
  assert.equal(env.scrolls.length, 1);
  env.h.unmount();
});

test("Keep editing focus return reveals the memory field without requiring another keyboard event", () => {
  for (const focusReturned of [false, true]) {
    const env = setup();
    env.focus();
    env.show();
    env.app("inactive");
    if (focusReturned) env.blur();
    env.resetField();
    if (focusReturned) env.focus();
    env.app("active");
    assert.equal(env.bottom(), 266);
    assert.equal(
      env.scrolls.length,
      2,
      "active remeasurement or focus return reveals the retained draft",
    );
    assert.ok(env.fieldBottom() <= 552 - spacing.lg);
    env.h.unmount();
  }
});

test("SettingsPage focus reveal honors Reduce Motion and works with the Android keyboard observer", () => {
  const env = setup({ platform: "android", reduced: true });
  env.focus();
  env.show();
  assert.equal(env.bottom(), 266);
  assert.equal(env.scrolls[0]?.animated, false);
  env.h.unmount();
});

test("late SettingsPage field measurements cannot scroll after blur, keyboard hide, background, scroll or unmount", () => {
  for (const boundary of [
    "blur",
    "hide",
    "background",
    "scroll",
    "unmount",
  ] as const) {
    const env = setup();
    env.delayMeasurement();
    env.focus();
    env.show();
    assert.ok(env.measurements.length > 0);
    if (boundary === "blur") env.blur();
    else if (boundary === "hide") env.hide();
    else if (boundary === "background") env.app("background");
    else if (boundary === "scroll") {
      invoke(env.scroll(), "onScrollBeginDrag", {});
      env.resetField();
    } else env.h.unmount();
    for (const measurement of env.measurements) measurement(20, 650, 340, 120);
    assert.deepEqual(env.scrolls, [], boundary);
    if (boundary !== "unmount") env.h.unmount();
  }
});

test("native offset updates during keyboard layout remeasure the focused field from the live scroll position", () => {
  const env = setup();
  env.delayMeasurement();
  env.focus();
  env.show();
  const old = env.measurements.shift()!;
  env.scrollFromNative(40);
  env.flushLayout();
  const current = env.measurements.shift()!;
  old(20, 650, 340, 120);
  assert.deepEqual(env.scrolls, []);
  current(20, 610, 340, 120);
  assert.deepEqual(env.scrolls, [
    { y: 650 + 120 - (552 - spacing.lg), animated: true },
  ]);
  env.h.unmount();
});

test("content growth while the keyboard stays open re-reveals the focused field", () => {
  const env = setup();
  env.focus();
  env.show();
  env.resetField();
  const initialScrolls = env.scrolls.length;
  env.contentSizeChange();
  assert.equal(env.scrolls.length, initialScrolls + 1);
  assert.ok(env.fieldBottom() <= 552 - spacing.lg);
  env.h.unmount();
});

for (const vector of [
  {
    name: "V1 iPhone sheet, English keyboard",
    view: { y: 174, height: 377 },
    keyboard: { screenY: 551, height: 301 },
    provider: { y: 70, height: 481 },
    windowHeight: 852,
    fieldY: 194,
  },
  {
    name: "V2 iPhone sheet, tall third-party keyboard",
    view: { y: 174, height: 283 },
    keyboard: { screenY: 457, height: 395 },
    provider: { y: 70, height: 387 },
    windowHeight: 852,
    fieldY: 194,
  },
  {
    name: "V3 iPad formSheet, system keyboard",
    view: { y: 161, height: 320 },
    keyboard: { screenY: 526, height: 498 },
    provider: { y: 57, height: 424 },
    windowHeight: 1024,
    fieldY: 181,
  },
]) {
  test(`KeyboardScrollView ${vector.name} never scrolls an already-visible autofocused Avatar field`, () => {
    const env = setup({ geometry: vector, avatar: true });
    const field = env
      .all()
      .find((node) => node.props.accessibilityLabel === "Avatar")!;
    assert.equal(field.props.autoFocus, true);
    assert.equal(field.props.multiline, false);
    env.resetField(vector.fieldY, 52);
    env.focus();
    env.show();
    assert.equal(env.bottom(), 0);
    assert.deepEqual(env.scrolls, []);
    assert.equal(env.fieldBottom(), vector.fieldY + 52);
    env.resize(vector.view.height);
    env.blur();
    env.focus();
    env.app("inactive");
    env.app("active");
    assert.equal(env.bottom(), 0);
    assert.deepEqual(
      env.scrolls,
      [],
      "layout, focus return and active remeasurement keep the field in place",
    );
    assert.equal(env.fieldBottom(), vector.fieldY + 52);
    env.h.unmount();
  });
}
