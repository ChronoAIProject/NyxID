import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { darkColors, lightColors } from "../../theme/mobileTheme";

for (const mode of ["light", "dark"] as const) {
  for (const state of ["idle", "ready", "pending"] as const) {
    test(`auth submit ${mode} ${state} retains primary, legibility, disabled semantics and correct feedback`, () => {
      const colors = mode === "light" ? lightColors : darkColors;
      const jsx = (type: any, props: any) => ({ type, props });
      const h = hookHarness<any>({
        "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
        "react-native": {
          Pressable: "Pressable",
          Text: "Text",
          View: "View",
          ActivityIndicator: "Spinner",
          StyleSheet: {
            create: (s: any) => s,
            absoluteFillObject: { position: "absolute" },
          },
        },
        "expo-linear-gradient": { LinearGradient: "Gradient" },
        "./useAuthTheme": { useAuthTheme: () => ({ mode, colors }) },
      });
      const Button = h.load(
        fileURLToPath(new URL("./AuthSubmitButton.tsx", import.meta.url)),
      ).AuthSubmitButton as any;
      h.mount(() =>
        Button({
          label: state === "pending" ? "Signing in..." : "Sign In",
          onPress() {},
          disabled: state === "idle",
          loading: state === "pending",
        }),
      );
      const p = h.value().props;
      assert.equal(p.disabled, state !== "ready");
      assert.deepEqual(JSON.parse(JSON.stringify(p.accessibilityState)), {
        disabled: state !== "ready",
        busy: state === "pending",
      });
      const content = p.children({ pressed: true }).props.children;
      const [gradient, edge, row] = content;
      assert.deepEqual(
        Array.from(gradient.props.colors),
        state === "ready"
          ? [colors.gradientStartPressed, colors.gradientEndPressed]
          : [colors.gradientStart, colors.gradientEnd],
      );
      assert.equal(
        gradient.props.style[1]?.opacity,
        state === "idle" && mode === "dark" ? 0.5 : undefined,
      );
      assert.equal(
        row.props.children[0]?.type === "Spinner",
        state === "pending",
      );
      assert.equal(
        row.props.children[1].props.style[1].color,
        colors.onPrimary,
      );
      assert.equal(
        edge.props.style[2].borderColor,
        state === "idle" ? colors.primary : "transparent",
      );
      assert.equal(
        Boolean(p.style({ pressed: true }).at(-1)?.transform),
        state === "ready",
      );
      h.unmount();
    });
  }
}
