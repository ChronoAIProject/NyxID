import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import {
  defaultChatLayout,
  normalizeChatLayout,
  type ChatLayout,
} from "./chatLayout";

type Element = {
  type: unknown;
  props: Record<string, any>;
  key?: number | string;
};
function elements(root: Element): Element[] {
  if (typeof root.type === "function" && root.type.name === "DynamicTypeText")
    return elements(root.type(root.props));
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}
const style = (node: Element) =>
  Object.assign({}, ...[node.props.style].flat(Infinity).filter(Boolean));
function setup(width = 402, fontScale = 3.12) {
  let layout = { ...defaultChatLayout };
  const jsx = (
    type: unknown,
    props: Element["props"],
    key?: number | string,
  ) => ({ type, props, key });
  const dynamicText = {};
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View",
      Text: "Text",
      Pressable: "Pressable",
      StyleSheet: { create: (value: unknown) => value },
      useWindowDimensions: () => ({ width, fontScale }),
    },
    "lucide-react-native": { Minus: "Minus", Plus: "Plus" },
    "../../components/DynamicTypeText": dynamicText,
    "./ChatBubbleSettings": { ChatBubbleSettings: "ChatBubbleSettings" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./ChatLayoutContext": {
      useChatLayout: () => ({
        layout,
        update: (patch: Partial<ChatLayout>) => {
          layout = normalizeChatLayout({ ...layout, ...patch });
        },
        reset: () => {
          layout = { ...defaultChatLayout };
        },
      }),
    },
  });
  Object.assign(
    dynamicText,
    harness.load(
      fileURLToPath(
        new URL("../../components/DynamicTypeText.tsx", import.meta.url),
      ),
    ),
  );
  const { ChatDisplaySettings } = harness.load(
    fileURLToPath(new URL("./ChatDisplaySettings.tsx", import.meta.url)),
  ) as {
    ChatDisplaySettings: () => Element;
  };
  harness.mount(ChatDisplaySettings);
  const all = () => elements(harness.value());
  const button = (label: string) => {
    const node = all().find((item) => item.props.accessibilityLabel === label);
    assert.ok(node, label);
    return node;
  };
  return {
    harness,
    all,
    button,
    layout: () => layout,
    scale: (next: number) => {
      fontScale = next;
      harness.render();
    },
  };
}

test("large-text Display keeps labels and values scalable while step icons fit their accessible 44pt targets", () => {
  for (const width of [320, 402, 800]) {
    const env = setup(width);
    const increase = env.button("Increase maximum composer height");
    const decrease = env.button("Decrease maximum composer height");
    for (const [button, glyph] of [
      [increase, "Plus"],
      [decrease, "Minus"],
    ] as const) {
      assert.equal(button.props.accessibilityRole, "button");
      assert.equal(style(button).width, 44);
      assert.equal(style(button).height, 44);
      assert.equal(button.props.children.type, glyph);
      assert.equal(button.props.children.props.size, 22);
      assert.equal(button.props.children.props.accessible, false);
    }
    const label = env
      .all()
      .find((item) => item.props.children === "Maximum composer height")!;
    assert.equal(style(label).width, "100%");
    for (const node of env.all().filter((item) => item.type === "Text")) {
      assert.notEqual(node.props.allowFontScaling, false);
      assert.equal(node.props.maxFontSizeMultiplier, undefined);
    }
    increase.props.onPress();
    env.harness.render();
    assert.equal(env.layout().composerMaxHeight, 140);
    env.button("Reset chat layout").props.onPress();
    env.harness.render();
    assert.equal(env.layout().composerMaxHeight, 120);
    env.harness.unmount();
  }
});

test("Display keeps bounds and column-width availability at standard text size", () => {
  const env = setup(800, 1);
  for (let step = 0; step < 20; step++) {
    const button = env.button("Increase maximum composer height");
    if (!button.props.disabled) button.props.onPress();
    env.harness.render();
  }
  assert.equal(env.layout().composerMaxHeight, 320);
  assert.equal(
    env.button("Increase maximum composer height").props.accessibilityState
      .disabled,
    true,
  );
  assert.ok(env.button("Increase chat column width"));
  env.harness.unmount();
});

test("live Display scaling refreshes text metrics while retaining row/control identity and preference state", () => {
  const env = setup(800, 1);
  env.button("Increase maximum composer height").props.onPress();
  env.harness.render();
  const identities = () =>
    env
      .all()
      .filter((node) => node.type !== "Text")
      .map((node) => [node.type, node.key]);
  const before = identities();
  assert.ok(
    env
      .all()
      .filter((node) => node.type === "Text")
      .every((node) => node.key === 1),
  );
  env.scale(2.143);
  assert.deepEqual(
    identities(),
    before,
    "card, rows, buttons and icons retain their React identity",
  );
  assert.ok(
    env
      .all()
      .filter((node) => node.type === "Text")
      .every((node) => node.key === 2.143),
  );
  assert.equal(env.layout().composerMaxHeight, 140);
  env.button("Increase maximum composer height").props.onPress();
  env.harness.render();
  assert.equal(env.layout().composerMaxHeight, 160);
  assert.ok(
    env
      .all()
      .filter((node) => node.type === "Text")
      .every((node) => node.key === 2.143),
    "preference edits do not replace text again",
  );
  env.scale(1);
  assert.ok(
    env
      .all()
      .filter((node) => node.type === "Text")
      .every((node) => node.key === 1),
  );
  env.harness.unmount();
});
