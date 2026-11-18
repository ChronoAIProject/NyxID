import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

test("account settings refresh only on focused foreground returns and reject callbacks from a replacement login", () => {
  let epoch = 1;
  let focus: () => () => void = () => () => {};
  let appState: (state: string) => void = () => {};
  let refreshed = 0;
  let cleared = 0;
  type Scope = (() => boolean) & {
    capture: () => { isCurrent: () => boolean; assertCurrent: () => void };
  };
  const harness = hookHarness<Scope>({
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
    ) => Scope;
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
  const inactiveTicket = harness.value().capture();
  const blur = focus();
  assert.equal(refreshed, 1);
  assert.equal(harness.value()(), true);
  assert.equal(inactiveTicket.isCurrent(), false);
  const firstActivation = harness.value().capture();
  assert.equal(firstActivation.isCurrent(), true);
  // Face ID, password AutoFill and Control Center only make iOS inactive.
  appState("inactive");
  appState("active");
  assert.equal(firstActivation.isCurrent(), true);
  assert.equal(cleared, 0);
  assert.equal(refreshed, 1);
  appState("background");
  assert.equal(harness.value()(), false);
  assert.equal(cleared, 1);
  appState("active");
  assert.equal(refreshed, 2);
  assert.equal(
    firstActivation.isCurrent(),
    false,
    "returning never revives the old ticket",
  );
  assert.throws(firstActivation.assertCurrent, { name: "AbortError" });
  const secondActivation = harness.value().capture();
  blur();
  appState("active");
  assert.equal(refreshed, 2, "a blurred settings screen must not re-activate");
  focus();
  assert.equal(
    secondActivation.isCurrent(),
    false,
    "blur permanently invalidates old work",
  );
  const accountTicket = harness.value().capture();
  epoch = 2;
  assert.equal(harness.value()(), false);
  assert.equal(accountTicket.isCurrent(), false);
  appState("active");
  assert.equal(refreshed, 3, "a new login cannot refresh old account settings");
  harness.unmount();
});
