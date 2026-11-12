import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

test("transcript measures at full height beneath a stable placeholder, then fades once and releases input", () => {
  for (const reducedMotion of [false, true]) {
    const animations: any[] = [];
    const values: number[] = [];
    const h = hookHarness<any>({
      "react/jsx-runtime": {
        jsx: (type: unknown, props: unknown) => ({ type, props }),
        jsxs: (type: unknown, props: unknown) => ({ type, props }),
      },
      "react-native": {
        View: "View",
        StyleSheet: { create: (v: unknown) => v, absoluteFill: {} },
        Animated: {
          View: "AnimatedView",
          Value: class {
            stopAnimation() {}
            setValue(value: number) {
              values.push(value);
            }
          },
          timing: (_value: unknown, config: unknown) => {
            animations.push(config);
            return { start() {}, stop() {} };
          },
        },
      },
      "../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => reducedMotion,
      },
      "../../theme/ThemeContext": {
        useTheme: () => ({ colors: { bg: "white" } }),
      },
      "../../components/Skeleton": { SkeletonRows: "Skeleton" },
    });
    const Viewport = h.load(
      fileURLToPath(new URL("./TranscriptViewport.tsx", import.meta.url)),
    ).TranscriptViewport as any;
    const props = { ready: false, children: "real list" };
    h.mount(() => Viewport(props));
    const list = () => h.value().props.children[0];
    assert.equal(h.value().props.style.flex, 1);
    assert.equal(list().props.style[0].flex, 1);
    assert.equal(list().props.style[0].minHeight, 0);
    assert.equal(list().props.pointerEvents, "none");
    assert.equal(list().props.importantForAccessibility, "no-hide-descendants");
    assert.equal(h.value().props.children[1].props.pointerEvents, "none");
    props.ready = true;
    h.render();
    assert.equal(list().props.pointerEvents, "auto");
    assert.equal(list().props.importantForAccessibility, "auto");
    assert.equal(h.value().props.children[1], false);
    assert.equal(animations.length, reducedMotion ? 0 : 1);
    if (reducedMotion) assert.equal(values.at(-1), 1);
    else assert.equal(animations[0].duration, 160);
    props.children = "updated list";
    h.render();
    assert.equal(
      animations.length,
      reducedMotion ? 0 : 1,
      "streaming and pagination never replay the opening fade",
    );
    h.unmount();
  }
});
