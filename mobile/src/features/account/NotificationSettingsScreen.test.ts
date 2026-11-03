import assert from "node:assert/strict";
import test from "node:test";
import { elements, setupSettingsScreen, style } from "./settingsTestHarness";

const setup = (connectedTelegram = false, pushDevices = 0) =>
  setupSettingsScreen(
    "./NotificationSettingsScreen.tsx",
    "NotificationSettingsScreen",
    { connectedTelegram, pushDevices },
  );

test("delivery rows keep live push and Telegram switches and stack under large text", () => {
  const env = setup(true, 1);
  const push = elements(env.row("Push Notifications")).find(
    (node) => node.type === "Switch",
  )!;
  const telegram = elements(env.row("Telegram")).find(
    (node) => node.type === "Switch",
  )!;
  assert.ok(push && telegram);
  assert.ok(env.text("@a_very_long_telegram_username"));
  push.props.onValueChange(true);
  telegram.props.onValueChange(true);
  assert.deepEqual(
    env.mutations.map((value) => JSON.parse(JSON.stringify(value))),
    [{ push_enabled: true }, { telegram_enabled: true }],
  );
  env.resize(402, 3.12);
  const body = env.parent(env.parent(env.parent(env.text("Push Notifications"))));
  assert.equal(style(body).flexDirection, "column");
  assert.equal(style(body).alignItems, "flex-start");
  assert.equal(style(body).maxWidth, "100%");
  assert.ok(
    elements(body).some((node) => node.type === "Switch"),
    "the switch stays with its stacked label",
  );
  env.harness.unmount();
});

test("an unlinked Telegram row opens the link sheet and push offers a 44pt enable action", () => {
  const env = setup(false, 0);
  const telegram = env.row("Telegram");
  assert.equal(telegram.type, "Pressable");
  assert.ok(elements(telegram).some((node) => node.type === "ChevronRight"));
  assert.ok(env.text("Link account"));
  telegram.props.onPress();
  env.harness.render();
  assert.equal(
    env.all().find((node) => node.type === "TelegramLinkModal")!.props.visible,
    true,
  );
  const enable = env.parent(env.text("Enable on this device"));
  assert.equal(enable.type, "Pressable");
  assert.ok(style(enable).minHeight >= 44);
  assert.equal(style(enable).maxWidth, "100%");
  env.harness.unmount();
});

test("approval default is one row with its value that opens an in-app menu", () => {
  const env = setup(true, 1);
  const row = env.row("Request approval for");
  assert.ok(
    env.text("Writes only (default)"),
    "the current mode is the row value",
  );
  assert.ok(elements(row).some((node) => node.type === "ChevronsUpDown"));
  const items = () =>
    env.all().filter((node) => node.props.accessibilityRole === "menuitem");
  assert.equal(items().length, 0, "options are not spread across the page");
  row.props.onPress();
  env.harness.flush();
  assert.deepEqual(
    items().map((item) => [
      item.props.accessibilityLabel,
      item.props.accessibilityState.checked,
    ]),
    [
      ["Writes only (default)", true],
      ["All requests", false],
      ["Off", false],
    ],
  );
  items()[1]!.props.onPress();
  env.harness.flush();
  assert.deepEqual(JSON.parse(JSON.stringify(env.mutations.at(-1))), {
    global_approval_mode: "all",
  });
  assert.equal(items().length, 0, "choosing closes the menu");
  env.row("Request approval for").props.onPress();
  env.harness.flush();
  env
    .all()
    .find((node) => node.props.accessibilityLabel === "Close menu")!
    .props.onPress();
  env.harness.flush();
  assert.equal(items().length, 0, "tapping outside closes it");
  assert.ok(env.text("Per-service and organization policies still apply."));
  env.harness.unmount();
});
