import assert from "node:assert/strict";
import test from "node:test";
import { lightColors } from "../../theme/mobileTheme";
import { elements, setupSettingsScreen, style } from "./settingsTestHarness";

const setup = (params?: Record<string, unknown>) =>
  setupSettingsScreen("./AccountSettingsScreen.tsx", "AccountSettingsScreen", {
    params,
  });

test("Settings is a menu: grouped icon rows with chevrons open sub-pages and hold no controls", () => {
  const env = setup();
  const groups = env
    .all()
    .filter((node) => node.props.accessibilityRole === "header")
    .map((node) => [node.props.children].flat(Infinity).join(""));
  assert.deepEqual(groups, ["ACCOUNT", "PREFERENCES", "SUPPORT"]);
  const routes = [
    ["Account & security", "AccountSecurity"],
    ["Usage & credits", "UsageCredits"],
    ["Notifications", "NotificationSettings"],
    ["Display", "DisplaySettings"],
    ["Privacy", "PrivacySettings"],
    ["Terms of Service", "TermsOfService"],
    ["Privacy Policy", "PrivacyPolicy"],
  ];
  for (const [label, route] of routes) {
    const row = env.row(label!);
    assert.equal(row.type, "Pressable", label);
    assert.equal(row.props.accessibilityRole, "button", label);
    assert.equal(row.props.accessibilityHint, `Opens ${label}`, label);
    const parts = elements(row).map((node) => node.type);
    assert.ok(parts.includes("ChevronRight"), `${label} has a chevron`);
    assert.ok(
      elements(row).some(
        (node) => style(node).width === 30 && style(node).height === 30,
      ),
      `${label} has a leading icon tile`,
    );
    row.props.onPress();
    assert.deepEqual(structuredClone(env.pushes.at(-1)), [route], label);
  }
  // Controls live on sub-pages, never on the menu.
  for (const type of [
    "Switch",
    "SegmentControl",
    "ChatDisplaySettings",
    "PrimaryButton",
  ]) {
    assert.equal(
      env.all().some((node) => node.type === type),
      false,
      type,
    );
  }
  assert.ok(env.text("Alex Example"));
  assert.ok(env.text("Signs in with Email/Password"));
  for (const label of ["Scan QR code", "Terminal login code"]) {
    assert.equal(
      env.all().some((node) => node.props.accessibilityLabel === label),
      false,
    );
  }
  env.harness.unmount();
});

test("Sign Out and Delete Account are centred actions, not navigation, and deletion sits in its own danger group", () => {
  const env = setup();
  for (const label of ["Sign Out", "Delete Account"]) {
    const row = env.row(label);
    assert.equal(row.type, "Pressable");
    assert.equal(
      elements(row).some((node) => node.type === "ChevronRight"),
      false,
      `${label} is an action, not a link`,
    );
    assert.equal(row.props.accessibilityHint, undefined, label);
  }
  assert.equal(style(env.text("Delete Account")).color, lightColors.danger);
  assert.equal(style(env.text("Sign Out")).color, lightColors.textPrimary);
  const danger = env
    .all()
    .find(
      (node) =>
        elements(node).includes(env.text("Delete Account")) &&
        style(node).borderRadius !== undefined &&
        style(node).borderColor !== undefined,
    )!;
  assert.equal(
    elements(danger).includes(env.text("Sign Out")),
    false,
    "Delete Account is segregated from Sign Out",
  );
  assert.ok(
    env.text(
      "Permanently deletes your account and its data. This cannot be undone.",
    ),
  );
  env.pending();
  assert.equal(env.row("Deleting...").props.disabled, true);
  assert.equal(env.row("Sign Out").props.disabled, true);
  env.harness.unmount();
});

test("deep links to a Settings area open its sub-page above the menu", () => {
  for (const [section, route] of [
    ["notifications", "NotificationSettings"],
    ["display", "DisplaySettings"],
    ["privacy", "PrivacySettings"],
  ]) {
    const env = setup({ section });
    assert.deepEqual(structuredClone(env.pushes), [[route]], section);
    env.harness.unmount();
  }
});

test("menu rows and identity reflow under large text without suppressing scaling", () => {
  const env = setup();
  const body = (label: string) => env.parent(env.parent(env.text(label)));
  assert.equal(style(body("Notifications")).flexDirection, "row");
  for (const [width, fontScale] of [
    [402, 3.12],
    [320, 1],
  ]) {
    env.resize(width!, fontScale!);
    assert.equal(style(body("Notifications")).flexDirection, "column");
    assert.equal(style(body("Notifications")).alignItems, "flex-start");
    const initials = env.text("AE");
    const avatar = env.parent(initials);
    assert.ok(style(avatar).height >= 44 * fontScale!);
    assert.equal(style(env.parent(avatar)).flexDirection, "column");
    assert.equal(
      style(env.parent(env.text("alex.long-address@example.test"))).width,
      "100%",
    );
    for (const node of env.all().filter((item) => item.type === "Text")) {
      assert.notEqual(node.props.allowFontScaling, false);
      assert.equal(node.props.maxFontSizeMultiplier, undefined);
      assert.equal(node.props.numberOfLines, undefined);
    }
  }
  env.resize(402, 1);
  assert.equal(style(body("Notifications")).flexDirection, "row");
  env.harness.unmount();
});

test("live scaling refreshes only text leaves and keeps the title heading", () => {
  const env = setup();
  const identities = () =>
    env
      .all()
      .filter((node) => node.type !== "Text")
      .map((node) => [node.type, node.key]);
  const before = identities();
  for (const scale of [2.143, 1, 2.143]) {
    env.resize(402, scale);
    assert.deepEqual(identities(), before);
    assert.ok(
      env
        .all()
        .filter((node) => node.type === "Text")
        .every((node) => node.key === scale),
    );
    const title = env.render(
      env.module.AccountSettingsTitle!({
        children: "Settings",
        tintColor: "purple",
      }),
    );
    assert.equal(title.type, "Text");
    assert.equal(title.key, scale);
    assert.equal(title.props.role, "heading");
    assert.equal(title.props.children, "Settings");
    assert.equal(style(title).color, "purple");
  }
  env.harness.unmount();
});
