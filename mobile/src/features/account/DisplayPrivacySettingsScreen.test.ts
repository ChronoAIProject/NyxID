import assert from "node:assert/strict";
import test from "node:test";
import { elements, setupSettingsScreen, style } from "./settingsTestHarness";

test("Display holds the appearance control, the system switch and chat layout settings", () => {
  const env = setupSettingsScreen(
    "./DisplaySettingsScreen.tsx",
    "DisplaySettingsScreen",
  );
  const appearance = () =>
    env
      .all()
      .find(
        (node) =>
          node.props.accessibilityRole === "tablist" &&
          node.props.accessibilityLabel === "Appearance",
      )!;
  assert.equal(style(appearance()).flexDirection, "row");
  assert.equal(
    env
      .all()
      .find((node) => node.props.accessibilityLabel === "Light appearance")!
      .props.accessibilityState.selected,
    true,
  );
  const system = elements(env.row("Use system setting")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(system.props.value, true);
  system.props.onValueChange(false);
  assert.deepEqual(env.preferences, ["light"]);
  assert.ok(env.all().some((node) => node.type === "ChatDisplaySettings"));
  env.resize(402, 3.12);
  assert.equal(style(appearance()).flexDirection, "column");
  env.harness.unmount();
});

test("Privacy holds both analytics switches with the retention note", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
  );
  const account = elements(env.row("Account permission enabled")).find(
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
      "Optional events require account permission and this device's choice. Security and billing records have separate retention rules.",
    ),
  );
  env.harness.unmount();
});
