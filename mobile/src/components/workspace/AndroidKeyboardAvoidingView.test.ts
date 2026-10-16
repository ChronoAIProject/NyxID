import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

type Frame = { screenY: number; height: number };
type Measure = (x: number, y: number, width: number, height: number) => void;
type Element = {
  props: {
    ref: { current: { measureInWindow: (callback: Measure) => void } | null };
    onLayout: () => void;
    style: [{ flex: number }, { paddingBottom: number }];
  };
};

function setup(initialKeyboard?: Frame, safeAreaTop = 0) {
  const keyboardListeners = new Map<
    string,
    (event: { endCoordinates: Frame }) => void
  >();
  let onDimensions: (() => void) | undefined;
  let metrics = initialKeyboard;
  const measurements: Measure[] = [];
  const harness = hookHarness<Element>({
    "react-native": {
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
      Keyboard: {
        metrics: () => metrics,
        addListener: (
          name: string,
          callback: (event: { endCoordinates: Frame }) => void,
        ) => {
          keyboardListeners.set(name, callback);
          return { remove: () => keyboardListeners.delete(name) };
        },
      },
      Dimensions: {
        addEventListener: (_name: string, callback: () => void) => {
          onDimensions = callback;
          return { remove: () => (onDimensions = undefined) };
        },
      },
    },
    "react/jsx-runtime": {
      jsx: (_type: unknown, props: unknown) => ({ props }),
    },
    "react-native-safe-area-context": {
      useSafeAreaInsets: () => ({
        top: safeAreaTop,
        bottom: 24,
        left: 0,
        right: 0,
      }),
    },
  });
  const { AndroidKeyboardAvoidingView } = harness.load(
    fileURLToPath(
      new URL("./AndroidKeyboardAvoidingView.tsx", import.meta.url),
    ),
  ) as { AndroidKeyboardAvoidingView: (props: object) => Element };
  const element = harness.mount(() => AndroidKeyboardAvoidingView({}));
  element.props.ref.current = {
    measureInWindow: (callback) => measurements.push(callback),
  };
  return {
    harness,
    measurements,
    keyboardListeners,
    bottom: () => harness.flush().props.style[1].paddingBottom,
    layout: () => harness.value().props.onLayout(),
    show: (frame: Frame) => {
      metrics = frame;
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
