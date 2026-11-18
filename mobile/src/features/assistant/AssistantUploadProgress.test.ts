import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import type { UploadStatus } from "./assistantUploadStatus";

for (const reduced of [false, true]) {
  test(`bottom upload loader ${reduced ? "stays still under Reduced Motion" : "pulses only for indeterminate states"} and hides from VoiceOver`, () => {
    let starts = 0,
      stops = 0;
    let status: UploadStatus = {
      text: "Waiting to upload…",
      loader: "indeterminate",
      progress: 0,
    };
    const h = hookHarness<any>({
      "react/jsx-runtime": {
        jsx: (type: unknown, props: unknown) => ({ type, props }),
      },
      "react-native": {
        View: "View",
        StyleSheet: { create: (s: unknown) => s },
        Animated: {
          View: "Bar",
          Value: class {
            value: number;
            constructor(value: number) {
              this.value = value;
            }
            setValue(value: number) {
              this.value = value;
            }
          },
          timing: () => ({}),
          sequence: () => ({}),
          loop: () => ({ start: () => starts++, stop: () => stops++ }),
        },
      },
      "../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => reduced,
      },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    });
    const C = h.load(
      fileURLToPath(new URL("./AssistantUploadProgress.tsx", import.meta.url)),
    ).AssistantUploadProgress as any;
    h.mount(() => C({ status }));
    assert.equal(starts, reduced ? 0 : 1);
    assert.equal(h.value().props.accessible, false);
    assert.equal(h.value().props.accessibilityElementsHidden, true);
    assert.equal(
      h.value().props.importantForAccessibility,
      "no-hide-descendants",
    );
    assert.equal(h.value().props.children.props.style[1].width, "40%");
    if (reduced)
      assert.equal(h.value().props.children.props.style[1].opacity.value, 0.55);
    status = { text: "Uploading 42%", loader: "determinate", progress: 42 };
    h.render();
    assert.equal(stops, reduced ? 0 : 1);
    assert.equal(h.value().props.children.props.style[1].width, "42%");
    assert.equal(h.value().props.children.props.style[1].opacity, 0.75);
    status = { loader: "none", progress: 100 };
    h.render();
    assert.equal(h.value(), null);
    assert.equal(starts, reduced ? 0 : 1);
    h.unmount();
  });
}
