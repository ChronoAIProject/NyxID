import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import type { KeyboardViewportFrame } from "./KeyboardViewport";

type Frame = { screenY: number; height: number };
type Measure = (x: number, y: number, width: number, height: number) => void;
type Element = {
  props: {
    ref: { current: { measureInWindow: (callback: Measure) => void } | null };
    onLayout: () => void;
    style: [{ flex: number }, { paddingBottom: number }];
  };
};

function setup(
  initialKeyboard?: Frame,
  safeAreaTop = 0,
  platform = "android",
  safeAreaFrame = { y: 0, height: 852 },
  initialWindowHeight = Math.max(
    852,
    initialKeyboard ? initialKeyboard.screenY + initialKeyboard.height : 0,
  ),
  inSheet = false,
) {
  const keyboardListeners = new Map<
    string,
    (event: { endCoordinates: Frame }) => void
  >();
  let onDimensions: (() => void) | undefined;
  let metrics = initialKeyboard;
  let windowHeight = initialWindowHeight;
  let screenHeight = initialWindowHeight;
  let state = "active";
  let onApp: ((state: string) => void) | undefined;
  const layouts: Array<() => void> = [];
  const measurements: Measure[] = [];
  let animations = 0;
  const viewports: (KeyboardViewportFrame | undefined)[] = [];
  const harness = hookHarness<Element>({
    "react-native": {
      View: "View",
      Platform: { OS: platform },
      StyleSheet: { create: (styles: unknown) => styles },
      AppState: {
        get currentState() {
          return state;
        },
        addEventListener: (_event: string, callback: (state: string) => void) => {
          onApp = callback;
          return { remove: () => { onApp = undefined; } };
        },
      },
      Keyboard: {
        isVisible: () => !!metrics,
        metrics: () => metrics,
        scheduleLayoutAnimation: () => { animations++; },
        addListener: (
          name: string,
          callback: (event: { endCoordinates: Frame }) => void,
        ) => {
          keyboardListeners.set(name, callback);
          return { remove: () => keyboardListeners.delete(name) };
        },
      },
      Dimensions: {
        get: (name: string) => {
          assert.ok(name === "window" || name === "screen");
          return { height: name === "screen" ? screenHeight : windowHeight };
        },
        addEventListener: (_name: string, callback: () => void) => {
          onDimensions = callback;
          return { remove: () => (onDimensions = undefined) };
        },
      },
    },
    "react/jsx-runtime": {
      jsx: (_type: unknown, props: unknown) => ({ props }),
    },
    "../../app/WorkspaceHeader": { useWorkspaceSheet: () => inSheet ? {} : null },
    "react-native-safe-area-context": {
      useSafeAreaFrame: () => safeAreaFrame,
      useSafeAreaInsets: () => ({
        top: safeAreaTop,
        bottom: 24,
        left: 0,
        right: 0,
      }),
    },
  }, {
    requestAnimationFrame: (callback: () => void) => {
      layouts.push(callback);
      return layouts.length;
    },
    cancelAnimationFrame: () => {},
  });
  const { KeyboardViewport } = harness.load(
    fileURLToPath(new URL("./KeyboardViewport.tsx", import.meta.url)),
  ) as { KeyboardViewport: (props: object) => Element };
  const element = harness.mount(() =>
    KeyboardViewport({
      onViewportChange: (frame: KeyboardViewportFrame | undefined) =>
        viewports.push(frame),
    }),
  );
  element.props.ref.current = {
    measureInWindow: (callback) => measurements.push(callback),
  };
  return {
    harness,
    measurements,
    keyboardListeners,
    animations: () => animations,
    provider: (frame: typeof safeAreaFrame) => { safeAreaFrame = frame; harness.render(); },
    bottom: () => harness.flush().props.style[1].paddingBottom,
    viewport: () => {
      const frame = viewports.at(-1);
      return frame ? { ...frame } : undefined;
    },
    layout: () => harness.value().props.onLayout(),
    show: (frame: Frame) => {
      metrics = frame;
      windowHeight = Math.max(windowHeight, frame.screenY + frame.height);
      screenHeight = Math.max(screenHeight, frame.screenY + frame.height);
      keyboardListeners.get("keyboardDidShow")!({ endCoordinates: frame });
    },
    hide: () => {
      metrics = undefined;
      keyboardListeners.get("keyboardDidHide")!({
        endCoordinates: { screenY: 900, height: 0 },
      });
    },
    resize: (frame = metrics) => {
      metrics = frame;
      onDimensions!();
    },
    hasDimensionListener: () => Boolean(onDimensions),
    app: (next: string, frame = metrics) => {
      metrics = frame;
      state = next;
      onApp!(next);
    },
    metrics: (frame: Frame | undefined) => { metrics = frame; },
    windowHeight: (height: number) => { windowHeight = height; },
    settledLayout: () => {
      for (const layout of layouts.splice(0)) layout();
    },
  };
}

test("actual Android wrapper measures overlap and releases it after keyboard dismissal", () => {
  const env = setup();
  env.show({ screenY: 580, height: 286 });
  env.measurements.shift()!(0, 92, 400, 774);
  assert.equal(env.bottom(), 286);
  env.hide();
  assert.equal(env.bottom(), 0);
  env.harness.unmount();
});

test("Android Fabric window coordinates include the live status-bar origin", () => {
  const env = setup(undefined, 52);
  env.show({ screenY: 572, height: 295 });
  env.measurements.shift()!(0, 64, 400, 751);
  assert.equal(
    env.bottom(),
    295,
    "the complete action row stays above the IME",
  );
  env.harness.unmount();
});

test("an already resized window consumes no additional inset with a status bar", () => {
  const env = setup(undefined, 52);
  env.show({ screenY: 572, height: 295 });
  env.measurements.shift()!(0, 64, 400, 456);
  assert.equal(
    env.bottom(),
    0,
    "frame and IME must use the same screen origin",
  );
  env.harness.unmount();
});

for (const [viewHeight, expectedOverlap] of [[482, 35], [447, 0]] as const) {
  test(`Android IME-resized window retains its keyboard and reserves ${expectedOverlap}pt of remaining overlap`, () => {
    const env = setup(undefined, 24, "android", { y: 0, height: 914 }, 914);
    const frame = { screenY: 563, height: 351 };
    env.windowHeight(563);
    env.metrics(frame);
    env.keyboardListeners.get("keyboardDidShow")!({ endCoordinates: frame });
    assert.equal(env.measurements.length, 1, "screen coordinates remain valid after native resize");
    env.measurements.shift()!(0, 92, 411, viewHeight);
    assert.equal(env.bottom(), expectedOverlap);
    assert.deepEqual(env.viewport(), { y: 92, height: viewHeight - expectedOverlap });
    env.resize();
    env.measurements.shift()!(0, 92, 411, viewHeight);
    assert.equal(env.bottom(), expectedOverlap, "resize events cannot erase the IME frame");
    env.hide();
    assert.equal(env.bottom(), 0);
    env.harness.unmount();
  });
}

test("a newer resized layout wins over a pending pre-resize measurement", () => {
  const env = setup();
  env.show({ screenY: 580, height: 286 });
  const beforeResize = env.measurements.shift()!;
  env.layout();
  env.measurements.shift()!(0, 92, 400, 488);
  beforeResize(0, 92, 400, 774);
  assert.equal(env.bottom(), 0, "adjustResize must not add another IME inset");
  env.harness.unmount();
});

test("keyboard hiding invalidates an outstanding native measurement", () => {
  const env = setup();
  env.show({ screenY: 580, height: 286 });
  const delayed = env.measurements.shift()!;
  env.hide();
  delayed(0, 92, 400, 774);
  assert.equal(env.bottom(), 0);
  env.harness.unmount();
});

test("mounting with a visible IME and later dimension changes use current metrics", () => {
  const env = setup({ screenY: 580, height: 286 });
  env.layout();
  env.measurements.shift()!(0, 92, 400, 774);
  assert.equal(env.bottom(), 286);
  env.resize({ screenY: 230, height: 170 });
  env.measurements.shift()!(0, 60, 800, 340);
  assert.equal(env.bottom(), 170);
  env.harness.unmount();
});

test("unmount removes all listeners and ignores pending measurement callbacks", () => {
  const env = setup();
  env.show({ screenY: 580, height: 286 });
  const delayed = env.measurements.shift()!;
  env.harness.unmount();
  delayed(0, 92, 400, 774);
  assert.equal(env.bottom(), 0);
  assert.equal(env.keyboardListeners.size, 0);
  assert.equal(env.hasDimensionListener(), false);
});

test("iOS measures the actual page or sheet origin without a guessed header or status-bar offset", () => {
  const env = setup(undefined, 59, "ios");
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 552, height: 300 },
  });
  env.measurements.shift()!(0, 119, 393, 699);
  assert.equal(
    env.bottom(),
    266,
    "only the page covered by the keyboard is reserved",
  );
  env.layout();
  env.measurements.shift()!(0, 119, 393, 699);
  assert.equal(
    env.bottom(),
    266,
    "padding does not accumulate when the child remeasures",
  );
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 852, height: 0 },
  });
  env.measurements.shift()!(0, 119, 393, 699);
  assert.equal(env.bottom(), 0);
  env.harness.unmount();
});

for (const [windowHeight, sheetHeight, keyboardHeight] of [
  [667, 637, 258],
  [956, 894, 328],
]) {
  test(`iOS ${windowHeight}pt partially resized sheet reserves its actual window overlap`, () => {
    const env = setup(
      undefined, 0, "ios", { y: 0, height: sheetHeight! }, windowHeight,
    );
    const expected = sheetHeight! - (windowHeight! - keyboardHeight!);
    env.show({
      screenY: windowHeight! - keyboardHeight!,
      height: keyboardHeight!,
    });
    env.measurements.shift()!(0, 44, 375, sheetHeight! - 44);
    assert.equal(env.bottom(), expected);
    env.layout();
    env.measurements.shift()!(0, 44, 375, sheetHeight! - 44);
    assert.equal(
      env.bottom(),
      expected,
      "the provider's shorter frame must not re-anchor the keyboard",
    );
    env.harness.unmount();
  });
}

test("an iOS drawer with an existing safe-area origin does not add that origin twice", () => {
  const env = setup(undefined, 0, "ios", { y: 70, height: 782 });
  env.show({ screenY: 552, height: 300 });
  env.measurements.shift()!(0, 119, 393, 699);
  assert.equal(env.bottom(), 266);
  env.harness.unmount();
});

test("foreground third-party keyboard frames cannot create huge, stale or duplicate page insets", () => {
  const env = setup({ screenY: 552, height: 300 }, 0, "ios");
  env.layout();
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 300);
  env.app("background");
  assert.equal(env.bottom(), 0);
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: -1200, height: 2052 },
  });
  assert.equal(env.bottom(), 0);
  assert.equal(env.measurements.length, 0);
  env.app("active", { screenY: 552, height: 300 });
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 300);
  // A stale did frame must match an actually superseded will frame. An unseen
  // did-only final frame is valid for third-party keyboards.
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 200, height: 652 },
  });
  env.measurements.shift()!(0, 92, 393, 760);
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 552, height: 300 },
  });
  env.measurements.shift()!(0, 92, 393, 760);
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 0, height: 1500 },
  });
  env.keyboardListeners.get("keyboardDidShow")!({
    endCoordinates: { screenY: 200, height: 652 },
  });
  assert.equal(env.measurements.length, 1, "stale did remeasures the accepted frame");
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 300);
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 600, height: 252 },
  });
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 252);
  env.keyboardListeners.get("keyboardDidShow")!({
    endCoordinates: { screenY: 200, height: 652 },
  });
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 252);
  env.metrics({ screenY: 200, height: 652 });
  env.settledLayout();
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 252);
  env.layout();
  env.measurements.shift()!(0, 92, 393, 508);
  assert.equal(env.bottom(), 0);
  env.harness.unmount();
});

test("iOS inactive system overlays preserve the visible keyboard inset through active remeasurement", () => {
  const env = setup({ screenY: 552, height: 300 }, 0, "ios");
  env.layout();
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 300);
  env.app("inactive");
  assert.equal(env.bottom(), 300);
  assert.equal(env.measurements.length, 0);
  env.app("active");
  assert.equal(env.bottom(), 300);
  env.measurements.shift()!(0, 92, 393, 760);
  env.settledLayout();
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 300);
  env.harness.unmount();
});

test("iOS background to inactive rehosting ignores oversized frames and active retains exactly the valid overlap", () => {
  const env = setup({ screenY: 552, height: 300 }, 0, "ios");
  env.layout();
  env.measurements.shift()!(0, 92, 393, 760);
  env.app("background");
  assert.equal(env.bottom(), 0);
  env.app("inactive");
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: -100, height: 1600 },
  });
  assert.equal(env.measurements.length, 0);
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 600, height: 252 },
  });
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 252);
  env.app("active", { screenY: 100, height: 752 });
  assert.equal(env.bottom(), 252);
  env.measurements.shift()!(0, 92, 393, 760);
  env.settledLayout();
  env.measurements.shift()!(0, 92, 393, 760);
  assert.equal(env.bottom(), 252);
  env.harness.unmount();
});

for (const vector of [
  {
    name: "V1 iPhone sheet, English keyboard",
    view: { y: 174, height: 377 },
    keyboard: { screenY: 551, height: 301 },
    provider: { y: 70, height: 481 },
    windowHeight: 852,
    overlap: 0,
    viewport: { y: 174, height: 377 },
  },
  {
    name: "V2 iPhone sheet, tall third-party keyboard",
    view: { y: 174, height: 283 },
    keyboard: { screenY: 457, height: 395 },
    provider: { y: 70, height: 387 },
    windowHeight: 852,
    overlap: 0,
    viewport: { y: 174, height: 283 },
  },
  {
    name: "V3 iPad formSheet, system keyboard",
    view: { y: 161, height: 320 },
    keyboard: { screenY: 526, height: 498 },
    provider: { y: 57, height: 424 },
    windowHeight: 1024,
    overlap: 0,
    viewport: { y: 161, height: 320 },
  },
  {
    name: "V4 full-screen page, root provider",
    view: { y: 120, height: 698 },
    keyboard: { screenY: 551, height: 301 },
    provider: { y: 0, height: 852 },
    windowHeight: 852,
    overlap: 267,
    viewport: { y: 120, height: 431 },
  },
]) {
  test(`${vector.name} measures overlap and visible viewport in window coordinates`, () => {
    const env = setup(
      undefined,
      0,
      "ios",
      vector.provider,
      vector.windowHeight,
    );
    env.show(vector.keyboard);
    env.measurements.shift()!(0, vector.view.y, 393, vector.view.height);
    assert.equal(env.bottom(), vector.overlap);
    assert.deepEqual(env.viewport(), vector.viewport);
    env.layout();
    env.measurements.shift()!(0, vector.view.y, 393, vector.view.height);
    assert.equal(
      env.bottom(),
      vector.overlap,
      "remeasurement cannot reserve the keyboard twice",
    );
    assert.deepEqual(env.viewport(), vector.viewport);
    env.harness.unmount();
  });
}

test("iPad windowed page uses the current root window and preserves its converted keyboard frame", () => {
  const env = setup(undefined, 0, "ios", { y: 0, height: 700 }, 700);
  env.show({ screenY: 484, height: 216 });
  env.measurements.shift()!(0, 80, 600, 586);
  assert.equal(env.bottom(), 182);
  assert.deepEqual(env.viewport(), { y: 80, height: 404 });
  env.resize();
  env.measurements.shift()!(0, 80, 600, 586);
  assert.equal(env.bottom(), 182);
  assert.deepEqual(env.viewport(), { y: 80, height: 404 });
  env.harness.unmount();
});

for (const trigger of [
  "keyboardDidShow",
  "keyboardDidChangeFrame",
  "provider",
] as const) {
  test(`fixed-height iPad sheet remeasures its lifted position after ${trigger}`, () => {
    const env = setup(undefined, 0, "ios", { y: 304, height: 860 }, 1366, true);
    const frame = { screenY: 1017, height: 349 };
    env.keyboardListeners.get("keyboardWillChangeFrame")!({
      endCoordinates: frame,
    });
    env.measurements.shift()!(0, 364, 720, 800);
    assert.equal(env.bottom(), 0, "pre-lift overlap is not committed");
    if (trigger === "provider") env.provider({ y: 157, height: 860 });
    else env.keyboardListeners.get(trigger)!({ endCoordinates: frame });
    env.measurements.shift()!(0, 217, 720, 800);
    assert.equal(env.bottom(), 0);
    assert.deepEqual(env.viewport(), { y: 217, height: 800 });
    assert.equal(env.animations(), 0);
    env.harness.unmount();
  });
}

test("V1 phone sheet never applies transient padding or reveal bounds before its keyboard lift", () => {
  const env = setup(undefined, 0, "ios", { y: 70, height: 782 }, 852, true);
  const frame = { screenY: 551, height: 301 };
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: frame,
  });
  env.measurements.shift()!(0, 174, 393, 644);
  assert.equal(env.bottom(), 0);
  assert.equal(
    env.viewport(),
    undefined,
    "do not reveal against pre-lift bounds",
  );
  env.provider({ y: 70, height: 481 });
  env.measurements.shift()!(0, 174, 393, 377);
  assert.equal(env.bottom(), 0);
  env.keyboardListeners.get("keyboardDidShow")!({ endCoordinates: frame });
  env.measurements.shift()!(0, 174, 393, 377);
  assert.equal(env.bottom(), 0);
  assert.deepEqual(env.viewport(), { y: 174, height: 377 });
  assert.equal(env.animations(), 0);
  env.harness.unmount();
});

test("full-screen will-change keeps the native keyboard animation and overlap", () => {
  const env = setup(undefined, 0, "ios");
  env.keyboardListeners.get("keyboardWillChangeFrame")!({
    endCoordinates: { screenY: 551, height: 301 },
  });
  env.measurements.shift()!(0, 120, 393, 698);
  assert.equal(env.bottom(), 267);
  assert.equal(env.animations(), 1);
  env.harness.unmount();
});

for (const trigger of [
  "keyboardDidShow",
  "keyboardDidChangeFrame",
  "onLayout",
]) {
  test(`sheet ${trigger} releases a will-frame hold even when the did frame is invalid`, () => {
    const env = setup(undefined, 0, "ios", { y: 0, height: 852 }, 852, true);
    env.keyboardListeners.get("keyboardWillChangeFrame")!({
      endCoordinates: { screenY: 551, height: 301 },
    });
    env.measurements.shift()!(0, 120, 393, 698);
    assert.equal(env.bottom(), 0, "will-frame overlap waits for native layout");
    if (trigger === "onLayout") env.layout();
    else
      env.keyboardListeners.get(trigger)!({
        endCoordinates: { screenY: -100, height: 1500 },
      });
    env.measurements.shift()!(0, 120, 393, 698);
    assert.equal(
      env.bottom(),
      267,
      "the next valid measurement uses the accepted keyboard frame",
    );
    assert.deepEqual(env.viewport(), { y: 120, height: 431 });
    env.harness.unmount();
  });
}
