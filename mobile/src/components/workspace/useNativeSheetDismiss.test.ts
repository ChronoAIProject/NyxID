import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";

function setup(platform = "ios", initial?: boolean) {
  let reduced = arguments.length > 1 ? initial : false;
  let closed = 0;
  let committed = 0;
  const h = hookHarness<any>({
    "react-native": { Platform: { OS: platform } },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotionPreference: () => reduced,
    },
  });
  const useSheet = h.load(
    fileURLToPath(new URL("./useNativeSheetDismiss.ts", import.meta.url)),
  ).useNativeSheetDismiss as any;
  h.mount(() => useSheet(() => closed++));
  return {
    h,
    value: () => h.value(),
    closed: () => closed,
    committed: () => committed,
    close: () => {
      h.value().close(() => committed++);
      h.flush();
    },
    reduced: (value: boolean) => {
      reduced = value;
      h.render();
    },
  };
}
for (const entry of [
  "selection Apply",
  "selection Cancel",
  "permissions Save",
  "permissions Cancel",
]) {
  test(`${entry} keeps its owner until native slide-down completes, including repeated presentations`, () => {
    for (let i = 0; i < 2; i++) {
      const x = setup();
      assert.equal(x.value().modalProps.visible, true);
      assert.equal(x.value().modalProps.animationType, "slide");
      x.close();
      assert.equal(x.value().modalProps.visible, false);
      assert.equal(x.closed(), 0);
      assert.equal(x.committed(), 0);
      x.close();
      x.value().modalProps.onDismiss();
      x.value().modalProps.onDismiss();
      assert.equal(x.closed(), 1);
      assert.equal(x.committed(), 1);
      x.h.unmount();
    }
  });
}
test("native sheets wait for the initial motion preference; reduced motion has no travel", async () => {
  const x = setup("ios", undefined);
  assert.equal(x.value().modalProps.visible, false);
  x.reduced(false);
  assert.equal(x.value().modalProps.visible, true);
  assert.equal(x.value().modalProps.animationType, "slide");
  x.h.unmount();
  const instant = setup("ios", true);
  assert.equal(instant.value().modalProps.animationType, "none");
  instant.close();
  await instant.h.settle();
  assert.equal(instant.closed(), 1);
  instant.h.unmount();
});
test("Android waits for native travel and an unmounted host cannot finish a stale dismissal", async () => {
  const x = setup("android");
  x.close();
  assert.equal(x.closed(), 0);
  await new Promise((resolve) => setTimeout(resolve, 370));
  assert.equal(x.closed(), 1);
  x.h.unmount();
  const gone = setup();
  gone.close();
  const finish = gone.value().modalProps.onDismiss;
  gone.h.unmount();
  finish();
  assert.equal(gone.closed(), 0);
});
