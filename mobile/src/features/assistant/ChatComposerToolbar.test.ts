import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

type E = { type: unknown; props: Record<string, any> };
const nodes = (node: any): E[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
    : [];

function load() {
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      ActivityIndicator: "ActivityIndicator",
      Animated: { View: "Animated.View", Value: class {} },
      StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
    },
    "lucide-react-native": {
      ArrowUp: "ArrowUp",
      AudioLines: "AudioLines",
      Plus: "Plus",
      Square: "Square",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../lib/accessibility/useReducedMotion": { useReducedMotion: () => false },
    "./AssistantMentionInput": { AssistantMentionInput: "AssistantMentionInput" },
    "./ComposerDictation": {
      ComposerDictationButton: "DictationButton",
      ComposerDictationReview: "DictationReview",
    },
  });
  h.mock(
    "./ChatComposer",
    h.load(fileURLToPath(new URL("./ChatComposer.tsx", import.meta.url))),
  );
  return h.load(
    fileURLToPath(new URL("./ChatComposerInput.tsx", import.meta.url)),
  ) as { ChatComposerToolbar: (props: any) => E };
}

function toolbar(compact: boolean | undefined, widths: number[][]) {
  const { ChatComposerToolbar } = load();
  const element = ChatComposerToolbar({
    attach: { label: "Attach", onPress() {} },
    mention: { type: "MentionButton", props: {} },
    settings: { type: "ComposerSettings", props: { hideTrigger: compact } },
    dictation: { active: false, phase: "idle" },
    dictationDisabled: false,
    layout:
      compact === undefined
        ? undefined
        : {
            compact,
            onLeadingWidth: (w: number) => widths.push([0, w]),
            onTrailingWidth: (w: number) => widths.push([1, w]),
          },
    children: { type: "SendButton", props: {} },
  });
  // Expand the ChatComposerRow function element to its native View.
  return (element.type as (p: any) => E)(element.props);
}

test("an idle composer hides mentions while retaining settings and held dictation controls", () => {
  const widths: number[][] = [];
  const twoRow = toolbar(false, widths);
  const oneRow = toolbar(true, widths);
  const shape = (root: E) =>
    [root.props.children].flat().map((child: any) =>
      child === null ? null : child.props.testID ?? child.type,
    );
  // Same slots in both layouts: no control remounts, so a held dictation
  // press still receives its release when the layout changes.
  assert.deepEqual(shape(twoRow), [
    "chat-composer-leading",
    "ComposerSettings",
    "View",
    "chat-composer-trailing",
  ]);
  assert.deepEqual(shape(oneRow), [
    "chat-composer-leading",
    "ComposerSettings",
    "View",
    "chat-composer-trailing",
  ]);
  const style = (root: E) => Object.assign({}, ...[root.props.style].flat().filter(Boolean));
  assert.equal(style(twoRow).position, undefined, "two rows: below the input");
  assert.equal(style(oneRow).position, "absolute", "one row: over the input line");
  assert.equal(style(oneRow).alignItems, "center");
  // The middle passes taps through to the input beneath.
  assert.equal(oneRow.props.pointerEvents, "box-none");
  const spacer = [oneRow.props.children].flat()[2] as E;
  assert.equal(spacer.props.pointerEvents, "none");
  const leading = nodes(oneRow).find((n) => n.props.testID === "chat-composer-leading")!;
  assert.ok(nodes(twoRow).some((n) => n.type === "MentionButton"));
  assert.equal(nodes(oneRow).some((n) => n.type === "MentionButton"), false);
  assert.equal(nodes(oneRow).find((n) => n.type === "ComposerSettings")!.props.hideTrigger, true);
  const trailing = nodes(oneRow).find((n) => n.props.testID === "chat-composer-trailing")!;
  assert.deepEqual(
    nodes(trailing).map((n) => n.type).filter((t) => t === "DictationButton" || t === "SendButton"),
    ["DictationButton", "SendButton"],
  );
  leading.props.onLayout({ nativeEvent: { layout: { width: 44 } } });
  trailing.props.onLayout({ nativeEvent: { layout: { width: 88 } } });
  assert.deepEqual(widths, [[0, 44], [1, 88]]);
});

test("without a layout the toolbar keeps today's two-row composer", () => {
  const root = toolbar(undefined, []);
  assert.equal(root.props.pointerEvents, "auto");
  assert.ok([root.props.children].flat().some((c: any) => c?.type === "ComposerSettings"));
});

test("touches on the composer pill mark themselves, so tapping send, attach or the text never blurs it", async () => {
  const { chatComposerFocus } = await import("./chatComposerFocus");
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      ActivityIndicator: "ActivityIndicator",
      Animated: {
        View: "Animated.View",
        Value: class {
          stopAnimation() {}
        },
        sequence: () => ({ start() {} }),
        timing: () => ({}),
        spring: () => ({}),
      },
      StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
    },
    "lucide-react-native": {
      ArrowUp: "ArrowUp",
      AudioLines: "AudioLines",
      Plus: "Plus",
      Square: "Square",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {}, mode: "light" }) },
    "../../lib/accessibility/useReducedMotion": { useReducedMotion: () => false },
  });
  const { ChatComposerFrame } = h.load(
    fileURLToPath(new URL("./ChatComposer.tsx", import.meta.url)),
  ) as { ChatComposerFrame: (props: any) => E };
  let blurs = 0;
  const input = { blur: () => blurs++ };
  chatComposerFocus.focused(input);
  for (const bounce of [true, false]) {
    h.mount(() => ChatComposerFrame({ bounce, children: "Composer" }));
    // Bubbling order: the pill's handler, then the workspace's.
    h.value().props.onTouchStart();
    chatComposerFocus.workspaceTouchStart();
    chatComposerFocus.workspaceTouchEnd();
    assert.equal(blurs, 0, `bounce ${bounce}`);
    h.unmount();
  }
  chatComposerFocus.blurred(input);
});
