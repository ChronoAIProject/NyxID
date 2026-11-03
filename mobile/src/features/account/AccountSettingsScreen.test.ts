import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";
import { renderSegmentControl } from "../../components/SegmentControl.testHarness";
import type { SegmentControlProps } from "../../components/SegmentControl";

type Element = { type: unknown; props: Record<string, any>; key?: string | number };
const style = (node: Element) =>
  Object.assign({}, ...[node.props.style].flat(Infinity).filter(Boolean));
function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}
function renderRows(node: Element, fontScale = 1): Element {
  if (node.type === renderSegmentControl) {
    return renderRows(
      renderSegmentControl(node.props as SegmentControlProps, { fontScale }),
      fontScale,
    );
  }
  if (
    typeof node.type === "function" &&
    ["AccountRow", "DynamicTypeText"].includes(node.type.name)
  ) {
    return renderRows(node.type(node.props), fontScale);
  }
  const render = (child: unknown): unknown =>
    Array.isArray(child)
      ? child.map(render)
      : child && typeof child === "object" && "props" in child
        ? renderRows(child as Element, fontScale)
        : child;
  return {
    ...node,
    props: { ...node.props, children: render(node.props.children) },
  };
}
function setup(connectedTelegram = false, pushDevices = 0) {
  let dimensions = { width: 402, height: 874, fontScale: 1, scale: 3 };
  let pending = false;
  const mutations: unknown[] = [];
  const pushes: unknown[][] = [];
  const jsx = (type: unknown, props: Element["props"], key?: number) => ({
    type,
    props,
    key,
  });
  const dynamicText = {};
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": {
      ...Object.fromEntries(
        ["View", "Text", "Image", "Pressable", "ScrollView", "Switch"].map(
          (name) => [name, name],
        ),
      ),
      StyleSheet: { create: (styles: unknown) => styles },
      useWindowDimensions: () => dimensions,
    },
    "react-native-svg": {
      __esModule: true,
      default: "Svg",
      Circle: "Circle",
      Path: "Path",
    },
    "@tanstack/react-query": {
      useQueryClient: () => ({}),
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        data:
          queryKey[1] === "profile"
            ? {
                display_name: "Alex Example",
                email: "alex.long-address@example.test",
                social_provider: null,
              }
            : queryKey[1] === "notificationSettings"
              ? {
                  push_device_count: pushDevices,
                  push_enabled: true,
                  telegram_connected: connectedTelegram,
                  telegram_enabled: true,
                  telegram_username: "a_very_long_telegram_username",
                  global_approval_mode: "writes_only",
                }
              : { enabled: true },
      }),
      useMutation: () => ({
        mutate: (value: unknown) => mutations.push(value),
        isPending: pending,
      }),
    },
    "../display/ChatDisplaySettings": {
      ChatDisplaySettings: "ChatDisplaySettings",
    },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "lucide-react-native": {
      ChevronRight: "ChevronRight",
      QrCode: "QrCode",
      Terminal: "Terminal",
    },
    "../../components/DynamicTypeText": dynamicText,
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../components/SegmentControl": { SegmentControl: renderSegmentControl },
    "../../components/OfflineBanner": { OfflineBanner: "OfflineBanner" },
    "../../components/TelegramLinkModal": {
      TelegramLinkModal: "TelegramLinkModal",
    },
    "../../components/ToastOverlay": { ToastOverlay: "ToastOverlay" },
    "../auth/AuthSessionContext": { useAuthSession: () => ({ signOut() {} }) },
    "../../lib/consent": {
      useMobileConsent: () => ({ accountId: "account", enabled: false }),
    },
    "../../lib/analyticsConsentState": {},
    "../../lib/analyticsRelay": {},
    "../../lib/api/http": {},
    "../../lib/telemetry": { capture() {} },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../lib/api/mobileApi": { mobileApi: {} },
    "../../lib/notifications/pushNotifications": {},
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: lightColors,
        mode: "light",
        preference: "system",
      }),
    },
    "../../theme/flowStyles": { createFlowStyles: () => ({}) },
  });
  Object.assign(
    dynamicText,
    harness.load(
      fileURLToPath(
        new URL("../../components/DynamicTypeText.tsx", import.meta.url),
      ),
    ),
  );
  const { AccountSettingsScreen, AccountSettingsTitle } = harness.load(
    fileURLToPath(new URL("./AccountSettingsScreen.tsx", import.meta.url)),
  ) as {
    AccountSettingsScreen: (props: any) => Element;
    AccountSettingsTitle: (props: any) => Element;
  };
  harness.mount(() =>
    renderRows(
      AccountSettingsScreen({
        navigation: { push: (...args: unknown[]) => pushes.push(args) },
      }),
      dimensions.fontScale,
    ),
  );
  const all = () => elements(harness.value());
  const text = (value: string) => {
    const node = all().find(
      (item) =>
        item.type === "Text" &&
        [item.props.children].flat(Infinity).join("") === value,
    );
    assert.ok(node, value);
    return node;
  };
  const parent = (child: Element) =>
    all().find((node) => [node.props.children].flat(Infinity).includes(child))!;
  return {
    all,
    text,
    parent,
    mutations,
    pushes,
    harness,
    title: () =>
      renderRows(
        AccountSettingsTitle({ children: "Settings", tintColor: "purple" }),
        dimensions.fontScale,
      ),
    pending: () => {
      pending = true;
      harness.render();
    },
    resize(width: number, fontScale: number) {
      dimensions = { ...dimensions, width, fontScale };
      harness.render();
    },
  };
}

test("Settings reflows label/value, identity and control rows as Dynamic Type grows without suppressing text scaling", () => {
  const env = setup();
  assert.equal(
    style(env.parent(env.text("Sign-in Method"))).flexDirection,
    "row",
  );
  const appearance = () => env.all().find((node) => node.props.accessibilityRole === "tablist" && node.props.accessibilityLabel === "Appearance")!;
  assert.equal(style(appearance()).flexDirection, "row");
  assert.equal(env.all().find((node) => node.props.accessibilityLabel === "Light appearance")!.props.accessibilityState.selected, true);
  for (const [width, fontScale] of [
    [402, 3.12],
    [320, 1],
  ]) {
    env.resize(width!, fontScale!);
    assert.equal(style(appearance()).flexDirection, "column", "Appearance follows the screen's large-text layout");
    for (const label of ["Light appearance", "Dark appearance"]) {
      assert.ok(elements(appearance()).some((node) => node.props.accessibilityRole === "tab" && node.props.accessibilityLabel === label));
    }
    for (const label of [
      "Sign-in Method",
      "Push Notifications",
      "Allow on this device",
      "Writes only (default)",
    ]) {
      const row = env.parent(env.text(label));
      assert.equal(style(row).flexDirection, "column", label);
      assert.equal(style(row).alignItems, "flex-start", label);
      assert.equal(style(row).height, undefined, label);
    }
    const value = env.text("Email/Password");
    assert.equal(style(value).textAlign, "left");
    assert.equal(style(env.parent(value)).maxWidth, "100%");
    const initials = env.text("AE");
    const avatar = env.parent(initials);
    assert.ok(style(avatar).height >= style(initials).lineHeight * fontScale!);
    assert.equal(style(env.parent(avatar)).flexDirection, "column");
    const identity = env.parent(env.text("alex.long-address@example.test"));
    assert.equal(style(identity).width, "100%");
    const enable = env.parent(env.text("Enable on this device"));
    assert.equal(style(enable).maxWidth, "100%");
    assert.ok(style(enable).minHeight >= 44);
    for (const node of env.all().filter((item) => item.type === "Text")) {
      assert.notEqual(node.props.allowFontScaling, false);
      assert.equal(node.props.maxFontSizeMultiplier, undefined);
      assert.equal(node.props.numberOfLines, undefined);
    }
  }
  env.resize(402, 1);
  assert.equal(style(appearance()).flexDirection, "row");
  assert.equal(
    style(env.parent(env.text("Sign-in Method"))).flexDirection,
    "row",
  );
  env.harness.unmount();
});

test("live Settings scaling refreshes title and label leaves while retaining scroll, controls and pending state", () => {
  const env = setup();
  const telegram = env
    .all()
    .find(
      (node) =>
        node.type === "Pressable" &&
        elements(node).some((child) => child.props.children === "Telegram"),
    )!;
  telegram.props.onPress();
  env.harness.render();
  env.pending();
  const identities = () =>
    env
      .all()
      .filter((node) => node.type !== "Text")
      .map((node) => [node.type, node.key]);
  const before = identities();
  for (const scale of [2.143, 1, 2.143]) {
    env.resize(402, scale);
    assert.deepEqual(
      identities(),
      before,
      "only Text leaves change their React key; scroll/control/modal identities remain",
    );
    assert.ok(
      env
        .all()
        .filter((node) => node.type === "Text")
        .every((node) => node.key === scale),
    );
    const title = env.title();
    assert.equal(title.type, "Text");
    assert.equal(title.key, scale);
    assert.equal(title.props.role, "heading");
    assert.equal(title.props.children, "Settings");
    assert.notEqual(title.props.allowFontScaling, false);
    assert.equal(style(title).color, "purple");
    assert.equal(
      env.all().find((node) => node.type === "TelegramLinkModal")!.props
        .visible,
      true,
      "screen-local state survives scaling",
    );
    const actions = env.all().filter((node) => node.type === "PrimaryButton");
    assert.ok(
      actions.every((node) => node.props.refreshLabelOnFontScaleChange),
      "every Settings action opts into text-only invalidation",
    );
    assert.equal(
      actions.find((node) => node.props.label === "Deleting...")!.props
        .disabled,
      true,
      "pending write state stays authoritative",
    );
  }
  env.harness.unmount();
});

test("large-text notification rows retain connected identity and live push/Telegram switches", () => {
  const env = setup(true, 1);
  env.resize(402, 3.12);
  const telegramIdentity = env.parent(env.text("Telegram"));
  assert.equal(style(telegramIdentity).flexDirection, "column");
  assert.equal(style(telegramIdentity).maxWidth, "100%");
  assert.equal(
    style(env.parent(env.text("@a_very_long_telegram_username"))).maxWidth,
    "100%",
  );
  const pushRow = env.parent(env.text("Push Notifications"));
  const push = elements(pushRow).find((node) => node.type === "Switch")!;
  const telegramRow = env.parent(telegramIdentity);
  const telegram = elements(telegramRow).find(
    (node) => node.type === "Switch",
  )!;
  assert.ok(push && telegram);
  push.props.onValueChange(true);
  telegram.props.onValueChange(true);
  assert.deepEqual(
    env.mutations.map((value) => JSON.parse(JSON.stringify(value))),
    [{ push_enabled: true }, { telegram_enabled: true }],
  );
  env.harness.unmount();
});

test("Settings keeps account identity and Usage while Quick actions live only in the sidebar", () => {
  const x = setup();
  const usage = x.all().find((node) => node.props.label === "Usage & credits");
  assert.ok(usage);
  usage.props.onPress();
  assert.deepEqual(structuredClone(x.pushes), [["UsageCredits"]]);
  assert.ok(x.text("Alex Example"));
  for (const label of ["Scan QR code", "Terminal login code"]) {
    assert.equal(
      x.all().some((node) => node.props.accessibilityLabel === label),
      false,
    );
  }
  x.harness.unmount();
});
