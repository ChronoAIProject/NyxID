import { setAppTranslationLanguage, translateApp } from "../i18n/appTranslation";
import { translate } from "../i18n/translate";
import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";
import { lightColors } from "../theme/mobileTheme";
import type { ToastState } from "./ToastOverlay";

type Element = { type: string; props: Record<string, any> };
function elements(node: Element): Element[] {
  return [node, ...[node.props.children].flat(Infinity).flatMap(
    (child) => child?.props ? elements(child) : [],
  )];
}

test("ToastOverlay defaults to one line and preserves each queued toast's explicit line count", () => {
  const animation = { duration: () => ({}) };
  const h = hookHarness<Element | null>({
    "react/jsx-runtime": {
      jsx: (type: string, props: object) => ({ type, props }),
      jsxs: (type: string, props: object) => ({ type, props }),
    },
    "react-native": {
      Text: "Text", View: "View", Pressable: "Pressable",
      StyleSheet: { create: (value: object) => value },
    },
    "react-native-reanimated": {
      __esModule: true, default: { View: "AnimatedView" },
      FadeIn: animation, FadeOut: animation, LinearTransition: animation,
    },
    "../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
  });
  const { ToastOverlay, localizedToast } = h.load(fileURLToPath(new URL("./ToastOverlay.tsx", import.meta.url)));
  let toast: ToastState = { message: "Existing caller", kind: "info" };
  try {
    h.mount(() => (ToastOverlay as (props: object) => Element | null)({ toast }));
    const text = (message: string) => elements(h.value()!).find(
      (node) => node.type === "Text" && node.props.children === message,
    )!;
    assert.equal(text("Existing caller").props.numberOfLines, 1);
    toast = { message: "Data export is not available on this server yet.", kind: "error", lines: 2 };
    h.render();
    assert.equal(text(toast.message).props.numberOfLines, 2);
    assert.equal(text("Existing caller").props.numberOfLines, 1);
    const name = 'Alice / 原样保留 / Save';
    toast = (localizedToast as (f:()=>ToastState)=>ToastState)(() => ({message:translateApp('Recent: {{name}}',{name}),kind:'info',action:{label:translateApp('Open'),onPress:()=>{}}}));
    h.render();
    for (const language of ['zh-CN','zh-TW','en'] as const) {
      setAppTranslationLanguage(language);h.render();
      assert.ok(text(translate(language,'Recent: {{name}}',{name})));
      assert.ok(text(translate(language,'Open')));
      assert.equal(elements(h.value()!).filter(n=>n.type==='AnimatedView').length,3,'changing language retains the same queued toasts');
    }
  } finally {
    setAppTranslationLanguage('en');
    h.unmount();
  }
});
