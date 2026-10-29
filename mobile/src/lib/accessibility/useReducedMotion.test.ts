import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import {
  hookHarness,
  deferred,
} from "../../features/assistant/assistantHookTestHarness";

test("motion preference is cached for newly mounted sheets and a native event wins a stale initial read", async () => {
  const initial = deferred<boolean>();
  let listener: (value: boolean) => void = () => {};
  const h = hookHarness<boolean | undefined>({
    "react-native": {
      AccessibilityInfo: {
        addEventListener: (_: string, next: typeof listener) => {
          listener = next;
          return { remove() {} };
        },
        isReduceMotionEnabled: () => initial.promise,
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useReducedMotion.ts", import.meta.url)),
  ).useReducedMotionPreference as () => boolean | undefined;
  h.mount(hook);
  assert.equal(h.value(), undefined);
  listener(false);
  h.flush();
  assert.equal(h.value(), false);
  initial.resolve(true);
  await h.settle();
  assert.equal(h.value(), false);
  h.unmount();
  h.mount(hook);
  assert.equal(
    h.value(),
    false,
    "first frame of a new sheet uses the known setting",
  );
  h.unmount();
});
