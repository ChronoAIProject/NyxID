import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import type { LayoutChangeEvent, TextInput } from "react-native";
import type { useLaunchInput } from "./launchPerformance";

test("login input readiness requires an enabled, laid-out current native node inside the window", async () => {
  let candidate:
    | { eligible: () => boolean; visible: () => Promise<boolean> }
    | undefined;
  const frames: (() => void)[] = [];
  const harness = hookHarness<ReturnType<typeof useLaunchInput>>(
    {
      "react-native": {
        AppState: { currentState: "active" },
        Dimensions: { get: () => ({ width: 390, height: 844 }) },
      },
      "./nativePerformance": { nativePerformance: null },
      "./metrics": {},
      "./launchRuntime": {
        createLaunchMonitor: () => ({
          input: (
            _key: object,
            _target: string,
            eligible: () => boolean,
            visible: () => Promise<boolean>,
          ) => {
            candidate = { eligible, visible };
            return () => {
              candidate = undefined;
            };
          },
        }),
      },
    },
    {
      requestAnimationFrame: (callback: () => void) => {
        frames.push(callback);
        return frames.length;
      },
      cancelAnimationFrame: () => {},
    },
  );
  function readCandidate() {
    return candidate;
  }
  let enabled = false;
  const { useLaunchInput: hook } = harness.load(
    fileURLToPath(new URL("./launchPerformance.ts", import.meta.url)),
  ) as { useLaunchInput: typeof useLaunchInput };
  harness.mount(() => hook("login_email", enabled, "login"));
  let location = 900;
  let point: ((x: number, y: number, w: number, h: number) => void) | undefined;
  const input = {
    measureInWindow: (cb: typeof point) => {
      point = cb;
    },
  } as unknown as TextInput;
  let current: TextInput | null = input;
  const layout = harness.value();
  layout(
    {
      nativeEvent: { layout: { width: 300, height: 40 } },
    } as LayoutChangeEvent,
    () => current,
  );
  harness.flush();
  assert.equal(candidate, undefined);
  enabled = true;
  harness.render();
  frames.splice(0).forEach((frame) => frame());
  const probe = readCandidate();
  assert.ok(probe);
  assert.ok(probe.eligible());
  const outside = probe.visible();
  point!(0, location, 300, 40);
  assert.equal(await outside, false);
  location = 100;
  const inside = probe.visible();
  point!(0, location, 300, 40);
  assert.equal(await inside, true);
  const stale = probe.visible();
  current = null;
  point!(0, location, 300, 40);
  assert.equal(await stale, false);
  enabled = false;
  harness.render();
  assert.equal(probe.eligible(), false);
  harness.unmount();
});
