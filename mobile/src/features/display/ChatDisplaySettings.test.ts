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

function setup(
  file: "./ChatDisplaySettings.tsx" | "./ChatLayoutEditScreen.tsx",
  {
    width = 402,
    initial = defaultChatLayout,
    key = "composerMaxHeight" as keyof ChatLayout,
  } = {},
) {
  let layout = { ...initial };
  const navigations: unknown[][] = [];
  const jsx = (
    type: unknown,
    props: Element["props"],
    elementKey?: number | string,
  ) => ({ type, props, key: elementKey });
  const dynamicText = {};
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View",
      Text: "Text",
      Pressable: "Pressable",
      StyleSheet: { create: (value: unknown) => value },
      useWindowDimensions: () => ({ width, fontScale: 1 }),
    },
    "@react-navigation/native": {
      useNavigation: () => ({
        navigate: (...args: unknown[]) => navigations.push(args),
      }),
    },
    "lucide-react-native": { Minus: "Minus", Plus: "Plus" },
    "../../components/DynamicTypeText": dynamicText,
    "./ChatBubbleSettings": { ChatBubbleSettings: "ChatBubbleSettings" },
    "./ChatPreview": { ChatPreview: "ChatPreview" },
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsPage: "SettingsPage",
      SettingsRow: "SettingsRow",
    },
    "../../theme/designTokens": { spacing: {}, typeScale: {} },
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
  const module = harness.load(
    fileURLToPath(new URL(file, import.meta.url)),
  ) as Record<string, (props: any) => Element>;
  const Screen =
    file === "./ChatDisplaySettings.tsx"
      ? module.ChatDisplaySettings!
      : module.ChatLayoutEditScreen!;
  harness.mount(() => Screen({ route: { params: { key } } }));
  const all = () => elements(harness.value());
  const find = (label: string) => {
    const node = all().find(
      (item) =>
        item.props.accessibilityLabel === label ||
        (item.type === "SettingsRow" && item.props.label === label),
    );
    assert.ok(node, label);
    return node;
  };
  return { harness, all, find, navigations, layout: () => layout };
}

test("chat layout values are rows with their value that open an edit page", () => {
  const env = setup("./ChatDisplaySettings.tsx");
  const composer = env.find("Maximum composer height");
  assert.equal(composer.props.value, "120 pt");
  composer.props.onPress();
  assert.deepEqual(JSON.parse(JSON.stringify(env.navigations)), [
    ["ChatLayoutEdit", { key: "composerMaxHeight" }],
  ]);
  assert.equal(
    env.all().some((node) => node.props.label === "Chat column width"),
    false,
    "column width is offered only on wide screens",
  );
  assert.equal(
    env.all().some((node) => node.props.label === "Reset Chat Layout"),
    false,
    "reset appears only once a value differs from its default",
  );
  env.harness.unmount();
  const wide = setup("./ChatDisplaySettings.tsx", {
    width: 800,
    initial: { ...defaultChatLayout, composerMaxHeight: 200 },
  });
  assert.equal(wide.find("Chat column width").props.value, "758 pt");
  wide.find("Reset Chat Layout").props.onPress();
  assert.deepEqual(wide.layout(), defaultChatLayout);
  wide.harness.unmount();
});

test("the edit page steps within bounds with 44pt targets and offers a reset", () => {
  const env = setup("./ChatLayoutEditScreen.tsx");
  assert.ok(env.all().some((node) => node.type === "ChatPreview"));
  const increase = () => env.find("Increase maximum composer height");
  for (const button of [increase(), env.find("Decrease maximum composer height")]) {
    const box = style({ ...button, props: { ...button.props, style: button.props.style({ pressed: false }) } });
    assert.equal(box.width, 44);
    assert.equal(box.height, 44);
    assert.equal(button.props.accessibilityRole, "button");
  }
  for (let step = 0; step < 20; step++) {
    if (!increase().props.disabled) increase().props.onPress();
    env.harness.render();
  }
  assert.equal(env.layout().composerMaxHeight, 320);
  assert.equal(increase().props.accessibilityState.disabled, true);
  env.find("Reset to 120 pt").props.onPress();
  env.harness.render();
  assert.equal(env.layout().composerMaxHeight, 120);
  assert.equal(
    env.all().some((node) => node.props.label === "Reset to 120 pt"),
    false,
  );
  env.harness.unmount();
});
