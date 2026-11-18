import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { translate } from "../../i18n/translate";
import type { LoginTerminal } from "./loginRequestResolver";
import { darkColors, lightColors } from "../../theme/mobileTheme";

type Element = { type: unknown; props: Record<string, unknown> };
const nodes = (root: Element): Element[] => [
  root,
  ...[root.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? nodes(child as Element)
        : [],
    ),
];
test("shared login terminal titles, explanations, Done and restart are localized and contain no decision controls", () => {
  for (const language of ["en", "zh-CN", "zh-TW"] as const)
    for (const state of [
      "approved",
      "denied",
      "expired",
      "completed",
      "unavailable",
    ] satisfies LoginTerminal[]) {
      const actions: string[] = [];
      const h = hookHarness<Element>({
        "react/jsx-runtime": {
          jsx: (type: unknown, props: Element["props"]) => ({ type, props }),
          jsxs: (type: unknown, props: Element["props"]) => ({ type, props }),
        },
        "react-native": { View: "View", Text: "Text" },
        "../../i18n/LanguageProvider": {
          useAppLanguage: () => ({
            t: (phrase: string) => translate(language, phrase),
          }),
        },
        "../../components/ScreenContainer": {
          ScreenContainer: "ScreenContainer",
        },
        "../../components/PrimaryButton": { PrimaryButton: "Button" },
        "./AuthSubmitButton": { AuthSubmitButton: "Button" },
        "../../components/icons/empty-state/MagicKeyIllustration": {
          MagicKeyIllustration: "Mark",
        },
        "../../components/icons/empty-state/RoadBarrierIllustration": {
          RoadBarrierIllustration: "Mark",
        },
        "./useAuthTheme": { useAuthTheme: () => ({ colors: {} }) },
        "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
      });
      const { LoginRequestTerminalView } = h.load(
        fileURLToPath(
          new URL("./LoginRequestTerminalView.tsx", import.meta.url),
        ),
      ) as { LoginRequestTerminalView: (props: unknown) => Element };
      h.mount(() =>
        LoginRequestTerminalView({
          state,
          inSheet: true,
          onDone: () => actions.push("done"),
          onRestart: () => actions.push("restart"),
        }),
      );
      const buttons = nodes(h.value()).filter((node) => node.type === "Button");
      assert.deepEqual(
        buttons.map((node) => node.props.label),
        [
          translate(language, "Done"),
          translate(language, "Scan or enter another code"),
        ],
      );
      for (const button of buttons) (button.props.onPress as () => void)();
      assert.deepEqual(actions, ["done", "restart"]);
      const copy = nodes(h.value())
        .filter((node) => node.type === "Text")
        .map((node) => node.props.children);
      assert.equal(copy.length, 2);
      if (language !== "en")
        for (const line of copy) assert.match(String(line), /[\u3400-\u9fff]/);
      h.unmount();
    }
});

test("terminal explanations stay centered without flex growth inside the mark-to-restart group", () => {
  for (const colors of [lightColors, darkColors]) {
    const h = hookHarness<Element>({
      "react/jsx-runtime": {
        jsx: (type: unknown, props: Element["props"]) => ({ type, props }),
        jsxs: (type: unknown, props: Element["props"]) => ({ type, props }),
      },
      "react-native": {
        View: "View",
        Text: "Text",
        StyleSheet: { create: (styles: unknown) => styles },
      },
      "../../components/ScreenContainer": {
        ScreenContainer: "ScreenContainer",
      },
      "../../components/PrimaryButton": { PrimaryButton: "Restart" },
      "./AuthSubmitButton": { AuthSubmitButton: "Done" },
      "../../components/icons/empty-state/MagicKeyIllustration": {
        MagicKeyIllustration: "Mark",
      },
      "../../components/icons/empty-state/RoadBarrierIllustration": {
        RoadBarrierIllustration: "Mark",
      },
      "./useAuthTheme": { useAuthTheme: () => ({ colors }) },
    });
    h.mock(
      "./deviceLoginStyles",
      h.load(fileURLToPath(new URL("./deviceLoginStyles.ts", import.meta.url))),
    );
    const { LoginRequestTerminalView } = h.load(
      fileURLToPath(new URL("./LoginRequestTerminalView.tsx", import.meta.url)),
    ) as {
      LoginRequestTerminalView: (props: unknown) => Element;
    };
    for (const state of [
      "approved",
      "denied",
      "expired",
      "completed",
      "unavailable",
    ] as const) {
      for (const inSheet of [false, true]) {
        h.mount(() =>
          LoginRequestTerminalView({
            state,
            inSheet,
            onDone() {},
            onRestart() {},
          }),
        );
        const group = h.value().props.children as Element;
        const children = group.props.children as Element[];
        assert.deepEqual(
          Array.from(children, (child) => child.type),
          ["Mark", "Text", "Text", "Done", "Restart"],
        );
        const body = children[2]!.props.style as Record<string, unknown>;
        assert.equal(body.flex, undefined);
        assert.equal(body.flexGrow ?? 0, 0);
        assert.equal(body.textAlign, "center");
        assert.equal(body.maxWidth, 360);
        assert.equal(body.color, colors.textSecondary);
        const layout = Object.assign({}, ...(group.props.style as object[]));
        assert.equal(layout.alignItems, "center");
        assert.equal(layout.justifyContent, "center");
      }
    }
    h.unmount();
  }
});

test("terminal Done and restart both expose the live disabled state during a pending decision", () => {
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: Element["props"]) => ({ type, props }),
      jsxs: (type: unknown, props: Element["props"]) => ({ type, props }),
    },
    "react-native": { View: "View", Text: "Text" },
    "../../components/ScreenContainer": { ScreenContainer: "Container" },
    "../../components/PrimaryButton": { PrimaryButton: "Button" },
    "./AuthSubmitButton": { AuthSubmitButton: "Button" },
    "../../components/icons/empty-state/RoadBarrierIllustration": {
      RoadBarrierIllustration: "Mark",
    },
    "../../components/icons/empty-state/MagicKeyIllustration": {
      MagicKeyIllustration: "Mark",
    },
    "./useAuthTheme": { useAuthTheme: () => ({ colors: {} }) },
    "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
  });
  const { LoginRequestTerminalView } = h.load(
    fileURLToPath(new URL("./LoginRequestTerminalView.tsx", import.meta.url)),
  ) as { LoginRequestTerminalView: (props: unknown) => Element };
  let disabled = true;
  h.mount(() =>
    LoginRequestTerminalView({
      state: "expired",
      inSheet: true,
      disabled,
      onDone() {},
      onRestart() {},
    }),
  );
  const buttons = () =>
    nodes(h.value()).filter((node) => node.type === "Button");
  assert.deepEqual(
    buttons().map((node) => node.props.disabled),
    [true, true],
  );
  disabled = false;
  h.render();
  assert.deepEqual(
    buttons().map((node) => node.props.disabled),
    [false, false],
  );
  h.unmount();
});
