import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";

function setup() {
  let fontResult: [boolean, Error | null] = [false, null];
  let timeout: (() => void) | undefined;
  const harness = hookHarness<boolean>({
    "expo-font": { useFonts: () => fontResult },
    "@expo-google-fonts/manrope": {},
    "@expo-google-fonts/space-grotesk": {},
    "@expo-google-fonts/jetbrains-mono": {},
  }, {
    setTimeout(callback: () => void, ms: number) {
      assert.equal(ms, 8000);
      timeout = callback;
      return 1;
    },
    clearTimeout() { timeout = undefined; },
  });
  const { useStartupFonts } = harness.load(
    fileURLToPath(new URL("./useStartupFonts.ts", import.meta.url)),
  ) as { useStartupFonts: () => boolean };
  harness.mount(useStartupFonts);
  return {
    harness,
    timeout: () => timeout,
    setResult(result: typeof fontResult) {
      fontResult = result;
      return harness.render();
    },
  };
}

test("loading fonts hold the splash until they load, fail, or time out", () => {
  for (const result of [[true, null], [false, new Error("unreadable font")]] as const) {
    const x = setup();
    assert.equal(x.harness.value(), false);
    assert.ok(x.timeout());
    assert.equal(x.setResult([result[0], result[1]]), true);
    assert.equal(x.timeout(), undefined);
    x.harness.unmount();
  }
  const x = setup();
  x.timeout()!();
  assert.equal(x.harness.flush(), true);
  x.harness.unmount();
  assert.equal(x.timeout(), undefined);
});
