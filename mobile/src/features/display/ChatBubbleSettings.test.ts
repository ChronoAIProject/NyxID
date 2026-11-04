import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import {
  lightBubblePalettes,
  type LightBubblePalette,
} from "../../theme/chatAppearance";
import { darkColors, lightColors } from "../../theme/mobileTheme";

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

test("preview follows resolved Light, Night and System appearance while each selected bubble retains its own palette", async () => {
  let scheme = "dark";
  let theme: any;
  const writes: [string, string][] = [];
  const jsx = (type: unknown, props: unknown) => ({ type, props });
  const harness = hookHarness<Element | null>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View",
      Text: "Text",
      Pressable: "Pressable",
      useColorScheme: () => scheme,
      useWindowDimensions: () => ({ fontScale: 1 }),
      StyleSheet: { create: (value: unknown) => value },
    },
    "expo-secure-store": {
      getItemAsync: async () => null,
      setItemAsync: async (key: string, value: string) => {
        writes.push([key, value]);
      },
    },
    "lucide-react-native": { Check: "Check" },
    "../../theme/ThemeContext": { useTheme: () => theme },
  });
  const dynamicText = harness.load(
    fileURLToPath(
      new URL("../../components/DynamicTypeText.tsx", import.meta.url),
    ),
  ) as { DynamicTypeText: (props: unknown) => Element };
  // Execute the real scalable text wrapper, keeping only native host components mocked.
  const renderText = (node: Element): Element => {
    if (node.type === dynamicText.DynamicTypeText)
      return renderText(dynamicText.DynamicTypeText(node.props));
    const render = (child: any): any =>
      Array.isArray(child)
        ? child.map(render)
        : child && typeof child === "object" && "props" in child
          ? renderText(child)
          : child;
    return {
      ...node,
      props: { ...node.props, children: render(node.props.children) },
    };
  };
  // The component imports this same wrapper via the normal relative module path.
  // Its native boundary is supplied by the harness when loaded below.
  const componentHarness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (value: unknown) => value },
    },
    "lucide-react-native": { Check: "Check" },
    "../../components/DynamicTypeText": dynamicText,
    "../../theme/ThemeContext": { useTheme: () => theme },
  });
  const { ChatBubbleSettings } = componentHarness.load(
    fileURLToPath(new URL("./ChatBubbleSettings.tsx", import.meta.url)),
  ) as { ChatBubbleSettings: () => Element };
  const { ThemeProvider } = harness.load(
    fileURLToPath(new URL("../../theme/ThemeContext.tsx", import.meta.url)),
  ) as { ThemeProvider: (props: unknown) => Element | null };
  harness.mount(() => {
    const provider = ThemeProvider({ children: null });
    if (!provider) return null;
    theme = provider.props.value;
    return provider;
  });
  await harness.settle();
  componentHarness.mount(() => renderText(ChatBubbleSettings()));
  for (const preference of ["light", "dark", "system"]) {
    theme.setPreference(preference);
    for (scheme of ["light", "dark"]) {
      harness.render();
      const colors =
        preference === "dark" || (preference === "system" && scheme === "dark")
          ? darkColors
          : lightColors;
      for (const key of Object.keys(
        lightBubblePalettes,
      ) as LightBubblePalette[]) {
        const allBefore = elements(componentHarness.render());
        const option = allBefore.find(
          (node) =>
            node.props.accessibilityLabel ===
            `${lightBubblePalettes[key].label} chat bubbles`,
        )!;
        option.props.onPress();
        harness.render();
        const all = elements(componentHarness.render());
        const parent = (child: Element) =>
          all.find((node) =>
            [node.props.children].flat(Infinity).includes(child),
          )!;
        const label = all.find(
          (node) => node.props.children === "Chat preview · light",
        )!;
        const canvas = parent(label);
        assert.equal(style(canvas).backgroundColor, colors.cardSoft);
        assert.equal(style(canvas).borderColor, colors.border);
        assert.equal(style(label).color, colors.textPrimary);
        const answer = all.find(
          (node) =>
            node.props.children ===
            "You can manage your connected services in Plugins.",
        )!;
        assert.equal(style(answer).color, colors.textPrimary);
        const message = all.find(
          (node) =>
            node.props.children === "Show me the services I have connected.",
        )!;
        assert.equal(
          style(parent(message)).backgroundColor,
          lightBubblePalettes[key].background,
        );
        assert.equal(style(message).color, lightBubblePalettes[key].foreground);
        assert.equal(
          all.find(
            (node) =>
              node.props.accessibilityLabel === option.props.accessibilityLabel,
          )!.props.accessibilityState.checked,
          true,
        );
      }
    }
  }
  await harness.settle();
  assert.ok(
    writes.some(
      ([key, value]) => key.includes("bubble") && value === "graphite",
    ),
  );
  componentHarness.unmount();
  harness.unmount();
});
