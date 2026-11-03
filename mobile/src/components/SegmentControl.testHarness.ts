import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";
import * as designTokens from "../theme/designTokens";
import { lightColors } from "../theme/mobileTheme";
import { createSegmentControlStyles } from "./segmentControlStyles";
import type { SegmentControlProps } from "./SegmentControl";

export type SegmentControlElement = {
  type: unknown;
  props: Record<string, any>;
  key?: string | number;
};

type Options = { fontScale?: number };

/** Runs the real grouped-choice component with isolated hooks and native hosts. */
export function setupSegmentControl(
  initialProps: SegmentControlProps,
  options: Options = {},
) {
  let props = initialProps;
  let fontScale = options.fontScale ?? 1;
  class Value {
    constructor(public value: number) {}
    setValue(next: number) {
      this.value = next;
    }
  }
  const jsx = (
    type: unknown,
    elementProps: SegmentControlElement["props"],
    key?: string | number,
  ) => ({ type, props: elementProps, key });
  const dynamicText = {};
  const harness = hookHarness<SegmentControlElement>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View",
      Text: "Text",
      Pressable: "Pressable",
      StyleSheet: { create: (styles: unknown) => styles },
      useWindowDimensions: () => ({
        width: 390,
        height: 844,
        scale: 3,
        fontScale,
      }),
      Animated: {
        View: "Animated.View",
        Value,
        spring: (value: Value, config: { toValue: number }) => ({
          start: () => value.setValue(config.toValue),
          stop: () => {},
        }),
      },
    },
    "../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../theme/designTokens": designTokens,
    "./segmentControlStyles": { createSegmentControlStyles },
    "./DynamicTypeText": dynamicText,
  });
  Object.assign(
    dynamicText,
    harness.load(
      fileURLToPath(new URL("./DynamicTypeText.tsx", import.meta.url)),
    ),
  );
  const { SegmentControl } = harness.load(
    fileURLToPath(new URL("./SegmentControl.tsx", import.meta.url)),
  ) as {
    SegmentControl: (props: SegmentControlProps) => SegmentControlElement;
  };
  harness.mount(() => SegmentControl(props));
  return {
    harness,
    tree: () => harness.value(),
    update: (next: SegmentControlProps) => {
      props = next;
      return harness.render();
    },
    layout: (width: number) => {
      harness.value().props.onLayout({ nativeEvent: { layout: { width } } });
      return harness.flush();
    },
    scale: (next: number) => {
      fontScale = next;
      return harness.render();
    },
  };
}

/** Parent screen tests expand the actual component without sharing hook slots. */
export function renderSegmentControl(
  props: SegmentControlProps,
  options?: Options,
): SegmentControlElement {
  const screen = setupSegmentControl(props, options);
  const tree = screen.tree();
  screen.harness.unmount();
  return tree;
}
