import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

test("the shared waveform follows bounded native samples, resizes and stays still under Reduce Motion", () => {
  let reduced = false;
  const listeners = new Set<(level: number) => void>();
  const subscribe = (listener: (level: number) => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  };
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { View: "View", StyleSheet: { create: (s: unknown) => s } },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { textSecondary: "ink" } }),
    },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => reduced,
    },
  });
  const { DictationWaveform } = h.load(
    fileURLToPath(new URL("./DictationWaveform.tsx", import.meta.url)),
  ) as { DictationWaveform: (props: any) => any };
  h.mount(() => DictationWaveform({ subscribe, height: 22 }));
  h.value().props.onLayout({ nativeEvent: { layout: { width: 15 } } });
  h.flush();
  const heights = () =>
    Array.from(h.value().props.children, (n: any) => n.props.style[1].height);
  for (const listener of listeners) {
    listener(-1);
    listener(0.5);
    listener(2);
  }
  h.flush();
  assert.deepEqual(heights(), [3, 12.5, 22]);
  h.value().props.onLayout({ nativeEvent: { layout: { width: 10 } } });
  h.flush();
  assert.deepEqual(heights(), [12.5, 22]);
  reduced = true;
  h.render();
  assert.equal(listeners.size, 0);
  assert.deepEqual(heights(), [3, 3]);
  reduced = false;
  h.render();
  assert.equal(listeners.size, 1);
  h.unmount();
  assert.equal(listeners.size, 0);
});
