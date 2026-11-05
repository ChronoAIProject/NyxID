import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { bubblePalettes, type BubblePalette } from "../../theme/chatAppearance";
import { darkColors, lightColors } from "../../theme/mobileTheme";
import { defaultChatLayout } from "./chatLayout";

type Element = { type: unknown; props: Record<string, any> };
const style = (node: Element) =>
  Object.assign({}, ...[node.props.style].flat(Infinity).filter(Boolean));
function elements(root: Element): Element[] {
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

test("Display keeps only compact Light/Dark samples and live palette choices through Light, Dark and System changes", async () => {
  let scheme = "dark";
  let theme: any;
  const writes: [string, string][] = [];
  const jsx = (type: unknown, props: unknown) => ({ type, props });
  const providerHarness = hookHarness<Element | null>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": { useColorScheme: () => scheme },
    "expo-secure-store": {
      getItemAsync: async () => null,
      setItemAsync: async (key: string, value: string) => {
        writes.push([key, value]);
      },
    },
  });
  const { ThemeProvider } = providerHarness.load(
    fileURLToPath(new URL("../../theme/ThemeContext.tsx", import.meta.url)),
  ) as { ThemeProvider: (props: unknown) => Element | null };
  providerHarness.mount(() => {
    const provider = ThemeProvider({ children: null });
    if (provider) theme = provider.props.value;
    return provider;
  });
  await providerHarness.settle();

  const dynamicText = {};
  const settings = {};
  const preview = {};
  const bubbles = {};
  const display = {};
  const ui = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": {
      ...Object.fromEntries(
        ["View", "Text", "Pressable", "Switch"].map((name) => [name, name]),
      ),
      useWindowDimensions: () => ({ width: 402, height: 874, fontScale: 1 }),
      StyleSheet: { create: (value: unknown) => value, hairlineWidth: 0.5 },
    },
    "@react-navigation/native": { useNavigation: () => ({ navigate() {} }) },
    "lucide-react-native": { Check: "Check", ChevronRight: "ChevronRight" },
    "../../theme/ThemeContext": { useTheme: () => theme },
    "../../components/DynamicTypeText": dynamicText,
    "../DynamicTypeText": dynamicText,
    "../ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../workspace/ScrollEdgeFades": { PageScrollView: "ScrollView" },
    "../../components/settings/SettingsList": settings,
    "../display/ChatPreview": preview,
    "./ChatBubbleSettings": bubbles,
    "../display/ChatDisplaySettings": display,
    "./ChatLayoutContext": {
      useChatLayout: () => ({ layout: defaultChatLayout, reset() {} }),
    },
    "../../lib/telemetry": { capture() {} },
  });
  const load = (path: string) =>
    ui.load(fileURLToPath(new URL(path, import.meta.url)));
  Object.assign(dynamicText, load("../../components/DynamicTypeText.tsx"));
  Object.assign(settings, load("../../components/settings/SettingsList.tsx"));
  Object.assign(preview, load("./ChatPreview.tsx"));
  Object.assign(bubbles, load("./ChatBubbleSettings.tsx"));
  Object.assign(display, load("./ChatDisplaySettings.tsx"));
  const { DisplaySettingsScreen } = load(
    "../account/DisplaySettingsScreen.tsx",
  ) as { DisplaySettingsScreen: (props: unknown) => Element };
  // Execute real current Settings, scalable text, palette rows and previews;
  // only native hosts, navigation, persistence and layout storage are boundaries.
  function render(node: Element): Element {
    if (typeof node.type === "function") return render(node.type(node.props));
    const child = (value: any): any =>
      Array.isArray(value)
        ? value.map(child)
        : value && typeof value === "object" && "props" in value
          ? render(value)
          : value;
    return {
      ...node,
      props: { ...node.props, children: child(node.props.children) },
    };
  }
  ui.mount(() => render(DisplaySettingsScreen({})));
  const all = () => elements(ui.value());
  const find = (label: string) => {
    const node = all().find((item) => item.props.accessibilityLabel === label);
    assert.ok(node, label);
    return node;
  };
  for (const preference of ["light", "dark", "system"]) {
    if (preference === "system")
      find("Automatic appearance").props.onValueChange(true);
    else
      find(
        `${preference === "light" ? "Light" : "Dark"} appearance`,
      ).props.onPress();
    for (scheme of ["light", "dark"]) {
      providerHarness.render();
      ui.render();
      assert.equal(theme.preference, preference);
      const effective = preference === "system" ? scheme : preference;
      assert.equal(theme.mode, effective);
      assert.equal(
        find("Automatic appearance").props.value,
        preference === "system",
      );
      assert.equal(
        find(`${effective === "light" ? "Light" : "Dark"} appearance`).props
          .accessibilityState.checked,
        true,
      );
      for (const key of Object.keys(bubblePalettes) as BubblePalette[]) {
        find(bubblePalettes[key].label).props.onPress();
        providerHarness.render();
        ui.render();
        assert.equal(theme.bubblePalette, key);
        assert.equal(
          find(bubblePalettes[key].label).props.accessibilityState.checked,
          true,
        );
        const frames = all().filter((node) =>
          /^Chat preview, (light|dark) appearance$/.test(
            node.props.accessibilityLabel ?? "",
          ),
        );
        assert.equal(
          frames.length,
          2,
          "no standalone light preview canvas returns",
        );
        for (const mode of ["light", "dark"] as const) {
          const colors = mode === "light" ? lightColors : darkColors;
          const frame = find(`Chat preview, ${mode} appearance`);
          const sample = elements(frame);
          const message = sample.find(
            (node) => node.props.children === "Hi NyxBot",
          )!;
          const bubble = sample.find((node) =>
            [node.props.children].flat().includes(message),
          )!;
          const answer = sample.find(
            (node) => node.props.children === "Here you go.",
          )!;
          assert.equal(style(frame).backgroundColor, colors.bg);
          assert.equal(style(frame).borderColor, colors.borderSoft);
          assert.equal(
            style(bubble).backgroundColor,
            bubblePalettes[key][mode].background,
          );
          assert.equal(
            style(message).color,
            bubblePalettes[key][mode].foreground,
          );
          assert.equal(style(answer).color, colors.textPrimary);
          assert.equal(
            sample.some(
              (node) =>
                node.props.children === "Show me my connected services.",
            ),
            false,
            "appearance tiles stay compact",
          );
          const tile = find(
            `${mode === "light" ? "Light" : "Dark"} appearance`,
          );
          assert.ok(
            style(tile).maxWidth <= 140,
            "light canvas is a bounded thumbnail",
          );
        }
      }
    }
  }
  await providerHarness.settle();
  assert.ok(
    writes.some(
      ([key, value]) => key.includes("bubble") && value === "graphite",
    ),
  );
  ui.unmount();
  providerHarness.unmount();
});
