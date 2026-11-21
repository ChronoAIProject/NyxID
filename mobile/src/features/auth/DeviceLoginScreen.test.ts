import assert from "node:assert/strict";
import test from "node:test";
import { StackActions } from "@react-navigation/routers";
import { translate } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import { formatAuthDeviceUserCode, normalizeAuthDeviceUserCode } from "./deviceUserCode";

type Element = { type: unknown; props: Record<string, any> };
function elements(root: Element): Element[] {
  return [root, ...[root.props.children].flat(Infinity).flatMap((child) =>
    child && typeof child === "object" && "props" in child ? elements(child) : [],
  )];
}

function setup({ inSheet = true, scanner = true, reduced = false, authenticated = true, userCode, previewRequest, decisionRequest, canGoBack = true, platform = "ios" }:
  { inSheet?: boolean; scanner?: boolean; reduced?: boolean; authenticated?: boolean; userCode?: string; canGoBack?: boolean;
    previewRequest?: (code: string) => Promise<unknown>;
    decisionRequest?: () => Promise<void>; platform?: "ios" | "android" } = {}) {
  let reducedMotion = reduced;
  let linkedCode = userCode;
  let language: Language = "en";
  const navigationCalls: unknown[] = [];
  const previews: { code: string }[] = [];
  const decisions: string[] = [];
  let keyboardVisible = false;
  let keyboardDismissals = 0;
  // Render the local ManualCodeModal with the actual screen; native boundaries
  // remain inert elements, so its hook and visible/animation props are exercised.
  const jsx = (type: any, props: Record<string, unknown>): Element =>
    typeof type === "function" ? type(props) : { type, props };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "@react-navigation/routers": { StackActions },
    "../../i18n/LanguageProvider": { useAppLanguage: () => ({ t: (phrase: string) => translate(language, phrase) }) },
    "react-native": {
      ...Object.fromEntries([
        "View", "Text", "TextInput", "Pressable", "ScrollView", "Modal",
        "KeyboardAvoidingView", "ActivityIndicator",
      ].map((name) => [name, name])),
      Platform: { OS: platform },
      Keyboard: {
        isVisible: () => keyboardVisible,
        dismiss: () => { keyboardDismissals++; keyboardVisible = false; },
      },
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
    "./AuthSubmitButton": { AuthSubmitButton: "PrimaryButton" },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/workspace/KeyboardViewport": { KeyboardViewport: "KeyboardViewport" },
    "../../components/icons/empty-state/MagicKeyIllustration": { MagicKeyIllustration: "MagicKeyIllustration" },
    "../../components/icons/empty-state/RoadBarrierIllustration": { RoadBarrierIllustration: "RoadBarrierIllustration" },
    "../../lib/accessibility/useReducedMotion": { useReducedMotion: () => reducedMotion },
    "../../lib/api/errorMessages": { resolveAuthDeviceErrorMessage: () => "Error" },
    "../../lib/api/mobileApi": { mobileApi: {
      previewAuthDevice: (code: string) => {
        previews.push({ code });
        return previewRequest?.(code) ?? Promise.reject(new Error("No preview configured"));
      },
      approveAuthDevice: async () => { decisions.push("approve"); await decisionRequest?.(); },
      denyAuthDevice: async () => { decisions.push("deny"); },
    } },
    "../../theme/designTokens": { spacing: { huge: 32 } },
    "./useAuthTheme": { useAuthTheme: () => ({ colors: {} }) },
    "./AuthTextInput": { AuthTextInput: "TextInput" },
    "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
    "./AuthSessionContext": { useAuthSession: () => ({ isAuthenticated: authenticated }) },
    "../../lib/api/agentKeyLoginApi": { agentKeyLoginApi: { preview: async () => { throw new Error("Not an Agent Key request"); } } },
    "../../lib/api/baseUrl": { getApiBaseUrl: () => "https://api.example.test/api/v1" },
    "./useUncertainLoginPreview": { useUncertainLoginPreview: () => ({ reset() {} }) },
    "./useLoginRequestResume": { useLoginRequestResume: () => {} },
    "./useLoginRequestHandoff": { useLoginRequestHandoff: () => () => true },
    "./useLoginPreviewCooldown": { useLoginPreviewCooldown: () => ({ coolingDown: false, sync() {} }) },

    "./useMfaStepUp": { useMfaStepUp: () => ({ busy: false, prompt: null }) },
    "./DeviceCodeScanner": { DeviceCodeScanner: "DeviceCodeScanner" },
    "./deviceUserCode": {
      formatAuthDeviceUserCode,
      normalizeAuthDeviceUserCode,
    },
  }, { Date });
  for (const name of ["./loginRequestErrors", "./loginRequestResolver", "./LoginRequestTerminalView"]) {
    harness.mock(name, harness.load(fileURLToPath(new URL(`${name}.${name.includes("View") ? "tsx" : "ts"}`, import.meta.url))));
  }
  const { DeviceLoginScreen } = harness.load(
    fileURLToPath(new URL("./DeviceLoginScreen.tsx", import.meta.url)),
  ) as { DeviceLoginScreen: (props: unknown) => Element };
  harness.mount(() => DeviceLoginScreen({
    navigation: {
      canGoBack: () => canGoBack,
      dispatch: (action: unknown) => navigationCalls.push(action),
      navigate: (name: string) => navigationCalls.push({ navigate: name }),
      goBack: () => navigationCalls.push({ back: true }),
      setParams: () => {},
    },
    route: { params: { start_scanner: scanner, user_code: linkedCode } },
  }));
  function find(type: string) {
    const found = elements(harness.value()).find((item) => item.type === type);
    assert.ok(found, `Missing ${type}`);
    return found;
  }
  return {
    harness, find, previews, decisions, navigationCalls,
    keyboard: (visible: boolean) => { keyboardVisible = visible; },
    keyboardDismissals: () => keyboardDismissals,
    setLanguage(value: Language) { language = value; harness.render(); },
    setCode(value: string) { linkedCode = value; harness.render(); },
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

test("Android manual-code Back hides the keyboard before closing and keeps the code", () => {
  const env = setup({ platform: "android" });
  env.find("DeviceCodeScanner").props.onManualEntry();
  env.harness.flush();
  const input = () => elements(env.harness.value()).find((item) => item.type === "TextInput" && item.props.accessibilityLabel === "Login code")!;
  input().props.onChangeText("ABCD-EFGH");
  env.harness.flush();
  env.keyboard(true);
  env.find("Modal").props.onRequestClose();
  env.harness.flush();
  assert.equal(env.keyboardDismissals(), 1);
  assert.equal(env.find("Modal").props.visible, true);
  assert.equal(input().props.value, "ABCD-EFGH");
  assert.deepEqual(env.navigationCalls, []);
  const scroller = elements(env.find("KeyboardViewport")).find((item) => item.type === "ScrollView")!;
  assert.equal(scroller.props.keyboardShouldPersistTaps, "handled");
  assert.equal(scroller.props.automaticallyAdjustKeyboardInsets, false);
  env.find("Modal").props.onRequestClose();
  env.harness.flush();
  assert.equal(env.find("Modal").props.visible, false);
  env.harness.unmount();
});

const pendingPreview = (label: string) => ({
  status: "pending",
  client_label: label,
  client_kind: "browser",
  seconds_remaining: 600,
  initiated_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 600_000).toISOString(),
});

test("opening a login link loads its review without approving or denying", async () => {
  const env = setup({ userCode: "abcd-efgh", previewRequest: async () => pendingPreview("Browser request") });
  await env.harness.settle();
  assert.deepEqual(env.previews.map((request) => request.code), ["ABCDEFGH"]);
  assert.equal(elements(env.harness.value()).some((item) => item.props.children === "Browser request"), true);
  assert.equal(elements(env.harness.value()).some((item) => item.type === "DeviceCodeScanner"), false);
  assert.deepEqual(env.decisions, []);
  env.harness.unmount();
});

test("a newer login link rejects the older preview response", async (context) => {
  let now = Date.now();
  context.mock.method(Date, "now", () => now);
  const first = deferred<unknown>();
  const second = deferred<unknown>();
  const env = setup({ userCode: "ABCD-EFGH", previewRequest: (code) => code === "ABCDEFGH" ? first.promise : second.promise });
  now += 1000;
  env.setCode("JKMN-PQRS");
  second.resolve(pendingPreview("New browser"));
  await env.harness.settle();
  first.resolve(pendingPreview("Old browser"));
  await env.harness.settle();
  const text = elements(env.harness.value()).map((item) => item.props.children);
  assert.equal(text.includes("New browser"), true);
  assert.equal(text.includes("Old browser"), false);
  assert.deepEqual(env.decisions, []);
  env.harness.unmount();
});

test("a signed-out link shows the request and requires sign-in before a decision", async () => {
  const env = setup({ authenticated: false, userCode: "ABCD-EFGH", previewRequest: async () => pendingPreview("Browser") });
  await env.harness.settle();
  const nodes = elements(env.harness.value());
  assert.ok(nodes.some((item) => item.props.label === "Sign in"));
  assert.equal(nodes.some((item) => item.props.label === "Approve full account session"), false);
  assert.deepEqual(env.decisions, []);
  env.harness.unmount();
});

test("a new link during approval waits for that decision, then can load its own review", async (context) => {
  let now = Date.now();
  context.mock.method(Date, "now", () => now);
  const decision = deferred<void>();
  const env = setup({ userCode: "ABCD-EFGH", previewRequest: async (code) => pendingPreview(code), decisionRequest: () => decision.promise });
  await env.harness.settle();
  now += 1000;
  const approve = elements(env.harness.value()).find((item) => item.props.label === "Approve full account session");
  assert.ok(approve);
  approve.props.onPress();
  env.setCode("JKMN-PQRS");
  await env.harness.settle();
  assert.equal(env.previews.length, 1, "a new link cannot replace an in-flight decision");
  decision.resolve();
  await env.harness.settle();
  assert.ok(elements(env.harness.value()).some((item) => item.props.children === "Login approved"));
  const done = elements(env.harness.value()).find((item) => item.props.label === "Done");
  assert.ok(done);
  now += 1000;
  done.props.onPress();
  await env.harness.settle();
  assert.deepEqual(env.previews.map(({code}) => code), ["ABCDEFGH", "JKMNPQRS"]);
  assert.deepEqual(env.decisions, ["approve"]);
  env.harness.unmount();
});

test("invalid and expired login links never become approvals", async () => {
  const invalid = setup({ userCode: "bad" });
  await invalid.harness.settle();
  assert.deepEqual(invalid.previews, []);
  assert.equal(elements(invalid.harness.value()).some((item) => item.props.children === "Enter a valid eight-character login code."), true);
  invalid.harness.unmount();
  const expired = setup({ userCode: "ABCD-EFGH", previewRequest: async () => ({ status: "expired" }) });
  await expired.harness.settle();
  assert.equal(elements(expired.harness.value()).some((item) => item.props.children === "This login request has expired. Start a new request on the requesting device."), true);
  assert.deepEqual(expired.decisions, []);
  expired.harness.unmount();
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

for (const status of ["pending", "denied", "invalid", "expired"]) {
  test(`abandoning a cold signed-out ${status} request replaces it with Auth`, async () => {
    const env = setup({
      authenticated: false, inSheet: false, scanner: false, canGoBack: false,
      userCode: status === "invalid" ? "bad" : "ABCD-EFGH",
      previewRequest: async () => ({ ...pendingPreview("Browser"), status }),
    });
    await env.harness.settle();
    const exit = elements(env.harness.value()).find((item) =>
      ["denied", "expired"].includes(status) ? item.props.label === "Done" : item.props.accessibilityLabel === "Back",
    );
    assert.ok(exit);
    exit.props.onPress();
    assert.deepEqual(env.navigationCalls, [StackActions.replace("Auth")]);
    assert.deepEqual(env.decisions, []);
    env.harness.unmount();
  });
}

test("intentional sign-in pushes Auth to preserve the request, while Back uses existing history", async () => {
  const env = setup({ authenticated: false, inSheet: false, scanner: false, userCode: "ABCD-EFGH", previewRequest: async () => pendingPreview("Browser") });
  await env.harness.settle();
  const signIn = elements(env.harness.value()).find((item) => item.props.label === "Sign in");
  assert.ok(signIn);
  signIn.props.onPress();
  assert.deepEqual(env.navigationCalls, [{ navigate: "Auth" }]);
  const back = elements(env.harness.value()).find((item) => item.props.accessibilityLabel === "Back");
  assert.ok(back);
  back.props.onPress();
  assert.deepEqual(env.navigationCalls.at(-1), { back: true });
  env.harness.unmount();
});

test("approved and denied completion copy follows the current app language together", async (context) => {
  let now = Date.now();
  context.mock.method(Date, "now", () => now);
  for (const approved of [true, false]) {
    const env = setup({ userCode: "ABCD-EFGH", previewRequest: async () => ({ ...pendingPreview("Browser"), status: approved ? "pending" : "denied" }) });
    await env.harness.settle();
    if (approved) {
      now += 1000;
      const approve = elements(env.harness.value()).find((item) => item.props.label === "Approve full account session");
      assert.ok(approve);
      approve.props.onPress();
      await env.harness.settle();
    }
    for (const language of ["en", "zh-CN", "zh-TW"] as const) {
      env.setLanguage(language);
      const nodes = elements(env.harness.value());
      const title = approved ? "Login approved" : "Request denied";
      assert.ok(nodes.some((item) => item.props.children === translate(language, title)));
      assert.ok(nodes.some((item) => item.props.label === translate(language, "Done")));
      if (approved) assert.ok(nodes.some((item) => item.props.children === translate(language, "Return to your browser to continue.")));
      if (language !== "en") assert.notEqual(translate(language, title), title, "completion needs a catalog entry");
    }
    env.harness.unmount();
  }
});

for (const kind of ["browser", "cli", "unknown"] as const) {
  test(`successful ${kind} approval shows the appropriate localized return instruction`, async (context) => {
    let now = Date.now();
    context.mock.method(Date, "now", () => now);
    const env = setup({ userCode: "ABCD-EFGH", previewRequest: async () => ({ ...pendingPreview("Requester"), client_kind: kind }) });
    await env.harness.settle();
    now += 1000;
    const approve = elements(env.harness.value()).find((item) => item.props.label === "Approve full account session");
    assert.ok(approve);
    approve.props.onPress();
    await env.harness.settle();
    const browser = "Return to your browser to continue.";
    const generic = "Return to where you started signing in.";
    const instruction = kind === "browser" ? browser : generic;
    for (const language of ["en", "zh-CN", "zh-TW"] as const) {
      env.setLanguage(language);
      const nodes = elements(env.harness.value());
      assert.ok(nodes.some((item) => item.props.children === translate(language, instruction)));
      assert.equal(nodes.some((item) => item.props.children === translate(language, kind === "browser" ? generic : browser)), false);
      if (language !== "en") assert.notEqual(translate(language, instruction), instruction);
    }
    assert.deepEqual(env.decisions, ["approve"]);
    env.harness.unmount();
  });
}
