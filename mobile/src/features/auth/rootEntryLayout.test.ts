import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
const flatten = (styles: any): Record<string, any> =>
  Array.isArray(styles)
    ? Object.assign({}, ...styles.map(flatten))
    : styles || {};

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

function setup(
  screenName: "DeviceLoginScreen" | "AgentKeyLoginScreen",
  inSheet: boolean,
  mfa: boolean,
) {
  const native = {
    ...Object.fromEntries(
      [
        "View",
        "Text",
        "TextInput",
        "Pressable",
        "ScrollView",
        "Modal",
        "KeyboardAvoidingView",
        "ActivityIndicator",
      ].map((name) => [name, name]),
    ),
    Platform: { OS: "ios" },
    StyleSheet: { create: <T>(styles: T) => styles },
  };
  const styleHarness = hookHarness({ "react-native": native });
  const { createDeviceLoginStyles } = styleHarness.load(
    fileURLToPath(new URL("./deviceLoginStyles.ts", import.meta.url)),
  ) as { createDeviceLoginStyles: (colors: unknown) => unknown };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": native,
    "react-native-safe-area-context": {
      useSafeAreaInsets: () => ({ top: 0, bottom: 0 }),
    },
    "lucide-react-native": {},
    "../../app/WorkspaceHeader": {
      useWorkspaceSheet: () => (inSheet ? {} : null),
      useWorkspaceBack: () => {},
    },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "../../components/settings/SettingsList": {},
    "../../components/workspace/WorkspaceSelectionField": {},
    "../../components/icons/empty-state/MagicKeyIllustration": {},
    "../../components/icons/empty-state/RoadBarrierIllustration": {},
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => true,
    },
    "../../lib/api/mobileApi": { mobileApi: {} },
    "../../lib/api/agentKeyLoginApi": { agentKeyLoginApi: {} },
    "../../lib/api/loginCodeApi": { loginCodeApi: {} },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./deviceLoginStyles": { createDeviceLoginStyles },
    "./AuthSessionContext": {
      useAuthSession: () => ({ isAuthenticated: inSheet }),
    },
    "./useMfaStepUp": {
      useMfaStepUp: () => ({ busy: false, prompt: mfa ? "MFA prompt" : null }),
    },
    "./DeviceCodeScanner": {},
    "./DeviceLoginScreen": {},
    "./AgentKeyCreateForm": {},
    "./LoginCodeStatusView": {},
  });
  const screen = harness.load(
    fileURLToPath(new URL(`./${screenName}.tsx`, import.meta.url)),
  )[screenName] as (props: unknown) => Element;
  harness.mount(() => screen({ navigation: {}, route: {} }));
  return harness;
}

for (const screen of ["DeviceLoginScreen", "AgentKeyLoginScreen"] as const) {
  test(`${screen} bounds root forms and MFA to a readable column while sheets own their width`, () => {
    for (const mfa of [false, true]) {
      for (const inSheet of [false, true]) {
        const harness = setup(screen, inSheet, mfa);
        const scroller = elements(harness.value()).find(
          (item) => item.type === "ScrollView",
        );
        assert.ok(scroller);
        const content = flatten(scroller.props.contentContainerStyle);
        assert.equal(content.maxWidth, inSheet ? undefined : 720);
        assert.equal(content.width, inSheet ? undefined : "100%");
        assert.equal(content.alignSelf, inSheet ? undefined : "center");
        assert.ok(
          content.padding > 0,
          "phone and narrow split-view gutters remain present",
        );
        const keyboard = elements(harness.value()).find(
          (item) => item.type === "KeyboardAvoidingView",
        );
        assert.equal(flatten(keyboard?.props.style).flex, 1);
        assert.equal(keyboard?.props.behavior, "padding");
        if (!mfa) {
          assert.equal(
            elements(harness.value()).some(
              (item) =>
                item.props.accessibilityRole === "button" &&
                item.props.accessibilityLabel === "Back",
            ),
            !inSheet,
          );
        }
        harness.unmount();
      }
    }
  });
}
