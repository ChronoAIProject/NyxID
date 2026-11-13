import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { authSurfaceColors, authLightColors } from "./authAppearance";
import { darkColors, lightColors } from "../../theme/mobileTheme";

for (const framed of [false, true])
  test(`auth ${framed ? "adorned" : "standalone"} field shares focus, preserves callbacks/ref and keeps dark styling`, () => {
    let mode = "light",
      invalid = false;
    const events: string[] = [];
    const ref = { current: null };
    const base = { borderColor: "original", height: 52 };
    const h = hookHarness<any>({
      "react/jsx-runtime": {
        jsx: (type: any, props: any) => ({ type, props }),
        jsxs: (type: any, props: any) => ({ type, props }),
      },
      "react-native": { TextInput: "TextInput", View: "View" },
      "./useAuthTheme": {
        useAuthTheme: () => ({
          mode,
          colors: authSurfaceColors(
            mode === "light" ? lightColors : darkColors,
            mode,
          ),
        }),
      },
    });
    const Field = h.load(
      fileURLToPath(new URL("./AuthTextInput.tsx", import.meta.url)),
    ).AuthTextInput as any;
    h.mount(() =>
      Field({
        style: base,
        frameStyle: framed ? base : undefined,
        leading: "Icon",
        trailing: "Reveal",
        invalid,
        ref,
        onFocus: () => events.push("focus"),
        onBlur: () => events.push("blur"),
        onChangeText: (value: string) => events.push(value),
      }),
    );
    const input = () => (framed ? h.value().props.children[1] : h.value());
    const chrome = () => h.value().props.style[1];
    assert.equal(input().props.ref, ref);
    assert.equal(input().props.placeholderTextColor, authLightColors.muted);
    input().props.onFocus({});
    h.flush();
    assert.equal(chrome().borderColor, authLightColors.focus);
    input().props.onChangeText("Draft");
    invalid = true;
    h.render();
    assert.equal(chrome().borderColor, undefined);
    input().props.onBlur({});
    h.flush();
    assert.deepEqual(events, ["focus", "Draft", "blur"]);
    mode = "dark";
    h.render();
    input().props.onFocus({});
    h.flush();
    assert.equal(chrome(), undefined);
    assert.equal(input().props.placeholderTextColor, undefined);
    assert.equal(input().props.selectionColor, undefined);
    if (framed) {
      assert.equal(h.value().props.children[0], "Icon");
      assert.equal(h.value().props.children[2], "Reveal");
    }
    h.unmount();
  });
