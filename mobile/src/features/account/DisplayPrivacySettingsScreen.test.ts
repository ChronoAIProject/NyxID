import assert from "node:assert/strict";
import test from "node:test";
import { elements, setupSettingsScreen, style } from "./settingsTestHarness";

test("Display previews appearance only in its Light/Dark tiles, with radios and the Automatic switch", () => {
  const env = setupSettingsScreen(
    "./DisplaySettingsScreen.tsx",
    "DisplaySettingsScreen",
  );
  const previews = env.all().filter((node) => node.type === "ChatPreview");
  assert.deepEqual(
    previews.map((node) => node.props.mode),
    ["light", "dark"],
    "no separate preview above the tiles",
  );
  const tile = (label: string) =>
    env.all().find((node) => node.props.accessibilityLabel === label)!;
  assert.equal(tile("Light appearance").props.accessibilityRole, "radio");
  assert.equal(tile("Light appearance").props.accessibilityState.checked, true);
  assert.equal(tile("Dark appearance").props.accessibilityState.checked, false);
  tile("Dark appearance").props.onPress();
  assert.deepEqual(env.preferences, ["dark"]);
  const automatic = elements(env.row("Automatic")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(automatic.props.value, true);
  automatic.props.onValueChange(false);
  assert.deepEqual(env.preferences, ["dark", "light"]);
  assert.ok(
    env.all().some((node) => node.type === "ChatDisplaySettings"),
    "bubble colour and chat layout render their own groups",
  );
  env.harness.unmount();
});

test("Privacy holds both analytics switches with the retention note", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
  );
  const account = elements(env.row("Allow for my account")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(
    account.props.accessibilityLabel,
    "Allow account usage analytics",
  );
  assert.equal(account.props.value, true);
  const device = elements(env.row("Allow on this device")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(
    device.props.accessibilityLabel,
    "Allow usage analytics on this device",
  );
  assert.equal(device.props.value, false);
  assert.ok(
    env.text(
      "Optional usage events are sent only when both your account and this device allow them. Security and billing records have separate retention rules.",
    ),
  );
  env.harness.unmount();
});
