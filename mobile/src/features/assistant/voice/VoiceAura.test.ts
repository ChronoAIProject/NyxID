import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistantHookTestHarness";
import * as auraState from "./voiceAuraState";
import type { VoiceAuraPhase } from "./voiceAuraState";

type Element = { type: unknown; key?: unknown; props: Record<string, any> };

function render(input: {
  phase: VoiceAuraPhase;
  reducedMotion: boolean;
  inputLevel?: number;
  outputLevel?: number;
  mode?: "light" | "dark";
}) {
  const calls: string[] = [];
  const shared: { value: unknown }[] = [];
  let index = 0;
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
    },
    "react-native": {
      View: "View",
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "react-native-reanimated": {
      __esModule: true,
      default: { View: "AnimatedView" },
      Easing: {
        inOut: (easing: unknown) => easing,
        sin: "sin",
        quad: "quad",
        linear: "linear",
      },
      cancelAnimation: () => calls.push("cancel"),
      useSharedValue: (value: unknown) => (shared[index++] ??= { value }),
      // Styles are evaluated once per render from the current shared values.
      useAnimatedStyle: (worklet: () => unknown) => worklet(),
      withRepeat: (animation: unknown) => {
        calls.push("repeat");
        return animation;
      },
      withTiming: (to: number) => to,
    },
    "react-native-svg": {
      __esModule: true,
      default: "Svg",
      Circle: "Circle",
      Defs: "Defs",
      RadialGradient: "RadialGradient",
      Stop: "Stop",
    },
    "../../../theme/ThemeContext": {
      useTheme: () => ({
        mode: input.mode ?? "dark",
        colors: {
          primary: "#C6B3FC",
          primaryLight: "#c4b5fd",
          gradientStart: "#8752CC",
          gradientEnd: "#5E00F5",
        },
      }),
    },
    "./voiceAuraState": auraState,
  });
  const { VoiceAura } = harness.load(
    fileURLToPath(new URL("./VoiceAura.tsx", import.meta.url)),
  ) as { VoiceAura: (props: unknown) => Element };
  const props = {
    phase: input.phase,
    input: { value: input.inputLevel ?? 0 },
    output: { value: input.outputLevel ?? 0 },
    reducedMotion: input.reducedMotion,
    size: 200,
  };
  const root = harness.mount(() => {
    index = 0;
    return VoiceAura(props);
  });
  const layers = (root.props.children as Element[]).flat();
  const transform = (layer: Element) =>
    Object.assign({}, ...(layer.props.style[2].transform as object[])) as {
      scale: number;
      translateX: number;
      translateY: number;
    };
  return { harness, root, layers, calls, transform };
}

test("the aura is decorative and hidden from assistive technology", () => {
  const { root, harness } = render({ phase: "resting", reducedMotion: false });
  assert.equal(root.props.accessible, false);
  assert.equal(root.props.accessibilityElementsHidden, true);
  assert.equal(root.props.importantForAccessibility, "no-hide-descendants");
  assert.equal(root.props.pointerEvents, "none");
  harness.unmount();
});

test("layered radial-gradient blobs breathe and orbit with motion allowed", () => {
  const { layers, calls, harness } = render({
    phase: "thinking",
    reducedMotion: false,
  });
  assert.equal(layers.length, 5);
  for (const layer of layers) {
    assert.equal(layer.type, "AnimatedView");
    const blob = layer.props.children as Element;
    const svg = (blob.type as (props: unknown) => Element)(blob.props);
    const gradient = [svg.props.children].flat(Infinity)[0] as Element;
    assert.equal(gradient.type, "Defs");
  }
  assert.deepEqual(
    calls.filter((call) => call === "repeat"),
    ["repeat", "repeat"],
  );
  harness.unmount();
});

test("listening grows with the microphone and speaking with playback", () => {
  const quiet = render({ phase: "listening", reducedMotion: false });
  const loud = render({
    phase: "listening",
    reducedMotion: false,
    inputLevel: 1,
  });
  assert.ok(
    loud.transform(loud.layers[0]!).scale >
      quiet.transform(quiet.layers[0]!).scale,
  );
  // Playback loudness does not move a listening aura.
  const echo = render({
    phase: "listening",
    reducedMotion: false,
    outputLevel: 1,
  });
  assert.equal(
    echo.transform(echo.layers[0]!).scale,
    quiet.transform(quiet.layers[0]!).scale,
  );
  const speaking = render({
    phase: "speaking",
    reducedMotion: false,
    outputLevel: 1,
  });
  const silent = render({ phase: "speaking", reducedMotion: false });
  assert.ok(
    speaking.transform(speaking.layers[0]!).scale >
      silent.transform(silent.layers[0]!).scale,
  );
  for (const f of [quiet, loud, echo, speaking, silent]) f.harness.unmount();
});

test("Reduce Motion is a static glow: no loops, orbit or level-driven scale", () => {
  const reduced = render({
    phase: "speaking",
    reducedMotion: true,
    outputLevel: 1,
  });
  assert.equal(reduced.calls.includes("repeat"), false);
  assert.equal(reduced.calls.includes("cancel"), true);
  for (const layer of reduced.layers) {
    const transform = reduced.transform(layer);
    assert.equal(Math.abs(transform.translateX), 0);
    assert.equal(Math.abs(transform.translateY), 0);
  }
  assert.equal(reduced.transform(reduced.layers[0]!).scale, 1);
  const thinking = render({ phase: "thinking", reducedMotion: true });
  assert.equal(
    thinking.transform(thinking.layers[0]!).scale,
    reduced.transform(reduced.layers[0]!).scale,
  );
  // Phases still differ, through opacity alone.
  const muted = render({ phase: "muted", reducedMotion: true });
  assert.ok(
    muted.layers[0]!.props.style[2].opacity <
      reduced.layers[0]!.props.style[2].opacity,
  );
  for (const f of [reduced, thinking, muted]) f.harness.unmount();
});

test("light and dark themes use their own violet stops", () => {
  const dark = render({ phase: "resting", reducedMotion: true, mode: "dark" });
  const light = render({
    phase: "resting",
    reducedMotion: true,
    mode: "light",
  });
  const colorsOf = (f: ReturnType<typeof render>) =>
    f.layers.map((layer) => (layer.props.children as Element).props.color);
  assert.notDeepEqual(colorsOf(dark), colorsOf(light));
  assert.ok(colorsOf(light).includes("#FFFFFF"));
  dark.harness.unmount();
  light.harness.unmount();
});
