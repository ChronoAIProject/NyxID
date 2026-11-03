import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

test("account settings refresh only on focused foreground returns and reject callbacks from a replacement login", () => {
  let epoch = 1;
  let focus: () => () => void = () => () => {};
  let appState: (state: string) => void = () => {};
  let refreshed = 0;
  let cleared = 0;
  const harness = hookHarness<() => boolean>({
    "@react-navigation/native": {
      useFocusEffect: (callback: typeof focus) => {
        focus = callback;
      },
    },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: (_event: string, callback: typeof appState) => {
          appState = callback;
          return { remove() {} };
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
  });
  const { useAccountSecurityRefresh } = harness.load(
    fileURLToPath(new URL("./useAccountSecurityRefresh.ts", import.meta.url)),
  ) as {
    useAccountSecurityRefresh: (
      refresh: () => void,
      clear: () => void,
    ) => () => boolean;
  };
  harness.mount(() =>
    useAccountSecurityRefresh(
      () => {
        refreshed++;
      },
      () => {
        cleared++;
      },
    ),
  );
  assert.equal(harness.value()(), false);
  const blur = focus();
  assert.equal(refreshed, 1);
  assert.equal(harness.value()(), true);
  appState("background");
  assert.equal(harness.value()(), false);
  assert.equal(cleared, 1);
  appState("active");
  assert.equal(refreshed, 2);
  blur();
  appState("active");
  assert.equal(refreshed, 2, "a blurred settings screen must not re-activate");
  focus();
  epoch = 2;
  assert.equal(harness.value()(), false);
  appState("active");
  assert.equal(refreshed, 3, "a new login cannot refresh old account settings");
  harness.unmount();
});
