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
  const headings = env
    .all()
    .filter((node) => node.props.accessibilityRole === "header");
  for (const heading of headings) {
    assert.equal(style(heading).textTransform, undefined);
    assert.equal(style(heading).fontSize, 11);
    assert.equal(style(heading).fontWeight, "500");
    assert.equal(style(heading).letterSpacing, 0.6);
  }
  assert.deepEqual(headings.map((node) => String(node.props.children).toUpperCase()), ["ACCOUNT", "PREFERENCES", "SUPPORT"]);
  assert.equal(headings[0]!.props.children, "ACCOUNT", "base uppercase presentation is retained");
  const routes = [
    ["Account & security", "AccountSecurity"],
    ["Usage & credits", "UsageCredits"],
    ["Notifications", "NotificationSettings"],
    ["Display", "DisplaySettings"],
    ["Language", "LanguageSettings"],
    ["Privacy", "PrivacySettings"],
    ["Terms of Service", "TermsOfService"],
    ["Privacy Policy", "PrivacyPolicy"],
    ["Open-source notices", "OpenSourceNotices"],
  ];
  for (const [label, route] of routes) {
    const row = env.row(label!);
    assert.equal(row.type, "Pressable", label);
    assert.equal(row.props.accessibilityRole, "button", label);
    assert.equal(row.props.accessibilityHint, `Opens ${label}`, label);
    const parts = elements(row).map((node) => node.type);
    assert.ok(parts.includes("ChevronRight"), `${label} has a chevron`);
    const tile = elements(row).find(
      (node) => style(node).width === 30 && style(node).height === 30,
    );
    assert.ok(tile, `${label} has a leading icon tile`);
    // Native grouped rows: a bare text-coloured glyph, no boxed tile.
    assert.equal(style(tile).backgroundColor, undefined, label);
    assert.equal(
      elements(tile).find(
        (node) => typeof node.type === "string" && node.props.size === 17,
      )!.props.color,
      lightColors.textPrimary,
      label,
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
  // Delete Account is a solid red button with a white label.
  assert.equal(style(env.text("Delete Account")).color, lightColors.onPrimary);
  assert.equal(
    style(env.row("Delete Account")).backgroundColor,
    lightColors.dangerSolid,
  );
  assert.equal(style(env.text("Sign Out")).color, lightColors.textPrimary);
  const danger = env
    .all()
    .find(
      (node) =>
        elements(node).includes(env.text("Delete Account")) &&
        style(node).borderRadius !== undefined,
    )!;
  assert.equal(
    elements(danger).includes(env.text("Sign Out")),
    false,
    "Delete Account is segregated from Sign Out",
  );
  assert.ok(
    env.text(
      "Permanently deletes your account and personal data. Necessary security and financial records are retained. This cannot be undone.",
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
  const body = (label: string) =>
    env.parent(env.parent(env.parent(env.text(label))));
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

test("Preferences always offers dictation; it is Voice only when live voice is enabled", () => {
  const off = setupSettingsScreen(
    "./AccountSettingsScreen.tsx",
    "AccountSettingsScreen",
  );
  assert.throws(() => off.row("Voice"));
  off.row("Dictation").props.onPress();
  assert.equal(
    JSON.stringify(off.pushes.at(-1)),
    '["VoiceSettings",{"voice":false}]',
  );
  off.harness.unmount();
  const on = setupSettingsScreen(
    "./AccountSettingsScreen.tsx",
    "AccountSettingsScreen",
    { voiceEnabled: true },
  );
  on.row("Voice").props.onPress();
  assert.equal(
    JSON.stringify(on.pushes.at(-1)),
    '["VoiceSettings",{"voice":true}]',
  );
  on.harness.unmount();
});

test("Apple Watch setup appears only on builds that include the watch app", () => {
  const without = setupSettingsScreen(
    "./AccountSettingsScreen.tsx",
    "AccountSettingsScreen",
  );
  assert.throws(() => without.row("Apple Watch"));
  without.harness.unmount();
  const withWatch = setupSettingsScreen(
    "./AccountSettingsScreen.tsx",
    "AccountSettingsScreen",
    { watchAvailable: true },
  );
  withWatch.row("Apple Watch").props.onPress();
  assert.equal(JSON.stringify(withWatch.pushes.at(-1)), '["AppleWatchSettings"]');
  withWatch.harness.unmount();
});

test("tablet account identity uses its measured card width when Dynamic Type outgrows the sheet", () => {
  const env = setup();
  env.resize(1032, 2.2);
  const identity = () => env.parent(env.parent(env.text("AE")));
  identity().props.onLayout({
    nativeEvent: { layout: { width: 688, height: 180 } },
  });
  env.harness.flush();
  assert.equal(style(identity()).flexDirection, "column");
  assert.equal(
    style(env.parent(env.text("alex.long-address@example.test"))).width,
    "100%",
  );
  env.resize(1194, 1);
  assert.equal(style(identity()).flexDirection, "row");
  env.harness.unmount();
});

test("Quick access (the Duo rail's list) is offered only where the window shows the rail", () => {
  const phone = setupSettingsScreen("./AccountSettingsScreen.tsx", "AccountSettingsScreen");
  // Ordinary iPhones and iPads have no rail, so no Quick access row.
  assert.throws(() => phone.row("Quick access"));
  phone.harness.unmount();
  const duo = setupSettingsScreen("./AccountSettingsScreen.tsx", "AccountSettingsScreen", {
    railWindow: true,
  });
  const row = duo.row("Quick access");
  assert.equal(row.props.accessibilityRole, "button");
  // Under Preferences, right after Language.
  const labels = duo
    .all()
    .filter((node) => node.type === "Pressable" && typeof node.props.accessibilityHint === "string")
    .map((node) => node.props.accessibilityHint);
  assert.ok(labels.indexOf("Opens Quick access") === labels.indexOf("Opens Language") + 1);
  row.props.onPress();
  assert.equal(JSON.stringify(duo.pushes.at(-1)), '["QuickAccessSettings"]');
  duo.harness.unmount();
});


test('M2622-03 sign-out waits for durability, blocks double taps and offers retry after failure', async () => {
  let resolve!: () => void, reject!: (reason: unknown) => void, calls = 0;
  const env = setupSettingsScreen('./AccountSettingsScreen.tsx', 'AccountSettingsScreen', {
    signOut: () => { calls++; return new Promise<void>((yes, no) => { resolve = yes; reject = no; }); },
  });
  try {
    const confirm = () => { env.row('Sign Out').props.onPress(); return env.alerts.at(-1)!.buttons![1]!.onPress!; };
    const press = confirm(); press(); press();
    await env.harness.settle();
    assert.equal(calls, 1);
    assert.equal(env.row('Signing out...').props.disabled, true);
    assert.equal(env.all().find(n => n.type === 'ToastOverlay')!.props.toast, null);
    reject(new Error('disk unavailable')); await env.harness.settle();
    assert.equal(env.row('Sign Out').props.disabled, false);
    assert.match(env.all().find(n => n.type === 'ToastOverlay')!.props.toast.message, /Couldn't sign out.*try again/);
    confirm()(); resolve(); await env.harness.settle();
    assert.equal(calls, 2);
    assert.equal(env.all().find(n => n.type === 'ToastOverlay')!.props.toast.message, 'You are signed out.');
  } finally { env.harness.unmount(); }
});
