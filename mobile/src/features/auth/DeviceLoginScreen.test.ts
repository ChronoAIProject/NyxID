import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
function elements(root: Element): Element[] {
  return [root, ...[root.props.children].flat(Infinity).flatMap((child) =>
    child && typeof child === "object" && "props" in child ? elements(child) : [],
  )];
}

function setup({ inSheet = true, scanner = true, reduced = false } = {}) {
  let reducedMotion = reduced;
  // Render the local ManualCodeModal with the actual screen; native boundaries
  // remain inert elements, so its hook and visible/animation props are exercised.
  const jsx = (type: any, props: Record<string, unknown>): Element =>
    typeof type === "function" ? type(props) : { type, props };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": {
      ...Object.fromEntries([
        "View", "Text", "TextInput", "Pressable", "ScrollView", "Modal",
        "KeyboardAvoidingView", "ActivityIndicator",
      ].map((name) => [name, name])),
      Platform: { OS: "ios" },
    },
    "react-native-safe-area-context": { useSafeAreaInsets: () => ({ top: 0, bottom: 0 }) },
    "lucide-react-native": Object.fromEntries([
      "ChevronDown", "ChevronLeft", "ChevronUp", "Info", "ScanQrCode", "X",
    ].map((name) => [name, name])),
    "../../app/WorkspaceHeader": {
      useWorkspaceSheet: () => inSheet ? {} : null,
      useWorkspaceBack: () => {},
    },
    "../../app/navigationHierarchy": { workspaceNavigateAction: () => ({}) },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/icons/empty-state/MagicKeyIllustration": { MagicKeyIllustration: "MagicKeyIllustration" },
    "../../components/icons/empty-state/RoadBarrierIllustration": { RoadBarrierIllustration: "RoadBarrierIllustration" },
    "../../lib/accessibility/useReducedMotion": { useReducedMotion: () => reducedMotion },
    "../../lib/api/errorMessages": { resolveAuthDeviceErrorMessage: () => "Error" },
    "../../lib/api/mobileApi": { mobileApi: {} },
    "../../theme/designTokens": { spacing: { huge: 32 } },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
    "./AuthSessionContext": { useAuthSession: () => ({ isAuthenticated: true }) },
    "./useMfaStepUp": { useMfaStepUp: () => ({ busy: false, prompt: null }) },
    "./DeviceCodeScanner": { DeviceCodeScanner: "DeviceCodeScanner" },
    "./deviceUserCode": {
      formatAuthDeviceUserCode: (value: string) => value,
      normalizeAuthDeviceUserCode: () => null,
    },
  });
  const { DeviceLoginScreen } = harness.load(
    fileURLToPath(new URL("./DeviceLoginScreen.tsx", import.meta.url)),
  ) as { DeviceLoginScreen: (props: unknown) => Element };
  harness.mount(() => DeviceLoginScreen({
    navigation: { canGoBack: () => true },
    route: { params: { start_scanner: scanner } },
  }));
  function find(type: string) {
    const found = elements(harness.value()).find((item) => item.type === type);
    assert.ok(found, `Missing ${type}`);
    return found;
  }
  return {
    harness, find,
    setReducedMotion(value: boolean) { reducedMotion = value; harness.render(); },
  };
}

test("manual code modal follows reduced motion on entry and while open", () => {
  for (const reduced of [false, true]) {
    const env = setup({ reduced });
    assert.equal(env.find("Modal").props.visible, false);
    env.find("DeviceCodeScanner").props.onManualEntry();
    env.harness.flush();
    assert.equal(env.find("Modal").props.visible, true);
    assert.equal(env.find("Modal").props.animationType, reduced ? "none" : "slide");
    env.setReducedMotion(!reduced);
    assert.equal(env.find("Modal").props.animationType, reduced ? "slide" : "none");
    const close = elements(env.harness.value()).find(
      (item) => item.type === "Pressable" && item.props.accessibilityLabel === "Close manual code entry",
    );
    assert.ok(close);
    close.props.onPress();
    env.harness.flush();
    assert.equal(env.find("Modal").props.visible, false);
    env.harness.unmount();
  }
});

test("QR sheet entry hides scanner Back and keeps manual entry dismissible", () => {
  const env = setup();
  assert.equal(env.find("DeviceCodeScanner").props.showHeader, false);
  env.find("DeviceCodeScanner").props.onManualEntry();
  env.harness.flush();
  assert.equal(elements(env.harness.value()).some(
    (item) => item.props.accessibilityLabel === "Back" || item.props.label === "Back",
  ), false);
  assert.ok(elements(env.harness.value()).some((item) => item.type === "X"));
  env.harness.unmount();
});

test("device login content suppresses Back only when rendered inside the sheet", () => {
  for (const inSheet of [false, true]) {
    const env = setup({ inSheet, scanner: false });
    assert.equal(elements(env.harness.value()).some(
      (item) => item.type === "Pressable" && item.props.accessibilityLabel === "Back",
    ), !inSheet);
    env.harness.unmount();
  }
});
