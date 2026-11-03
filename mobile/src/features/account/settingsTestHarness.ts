import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { lightColors } from "../../theme/mobileTheme";
import { renderSegmentControl } from "../../components/SegmentControl.testHarness";
import type { SegmentControlProps } from "../../components/SegmentControl";

export type Element = {
  type: unknown;
  props: Record<string, any>;
  key?: string | number;
};

export const style = (node: Element) =>
  Object.assign({}, ...[node.props.style].flat(Infinity).filter(Boolean));

export function elements(root: Element): Element[] {
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

// Settings building blocks render inline so tests see their real rows.
const EXPANDED = [
  "DynamicTypeText",
  "SettingsPage",
  "SettingsGroup",
  "SettingsRow",
  "SettingsMenuRow",
];

function renderRows(node: Element, fontScale: number): Element {
  if (node.type === renderSegmentControl) {
    return renderRows(
      renderSegmentControl(node.props as SegmentControlProps, { fontScale }),
      fontScale,
    );
  }
  if (typeof node.type === "function" && EXPANDED.includes(node.type.name)) {
    return renderRows(
      (node.type as (props: object) => Element)(node.props),
      fontScale,
    );
  }
  const render = (child: unknown): unknown =>
    Array.isArray(child)
      ? child.map(render)
      : child && typeof child === "object" && "props" in child
        ? renderRows(child as Element, fontScale)
        : child;
  // Pressable styles may be functions of the pressed state.
  const props =
    typeof node.props?.style === "function"
      ? { ...node.props, style: node.props.style({ pressed: false }) }
      : node.props;
  return { ...node, props: { ...props, children: render(props?.children) } };
}

export function setupSettingsScreen(
  file: string,
  exportName: string,
  {
    connectedTelegram = false,
    pushDevices = 0,
    params,
  }: {
    connectedTelegram?: boolean;
    pushDevices?: number;
    params?: Record<string, unknown>;
  } = {},
) {
  let dimensions = { width: 402, height: 874, fontScale: 1, scale: 3 };
  let pending = false;
  const mutations: unknown[] = [];
  const pushes: unknown[][] = [];
  const preferences: unknown[] = [];
  const sheets: Array<{ options: string[]; pick: (index: number) => void }> = [];
  const jsx = (type: unknown, props: Element["props"], key?: number) => ({
    type,
    props,
    key,
  });
  const dynamicText = {};
  const screenContainer = { ScreenContainer: "ScreenContainer" };
  const settingsList = {};
  const theme = {
    useTheme: () => ({
      colors: lightColors,
      mode: "light",
      preference: "system",
      setPreference: (value: unknown) => preferences.push(value),
    }),
  };
  const lucide = new Proxy(
    {},
    { get: (_target, name) => (name === "__esModule" ? false : String(name)) },
  );
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": {
      ...Object.fromEntries(
        [
          "View",
          "Text",
          "Image",
          "Pressable",
          "ScrollView",
          "Switch",
          "TextInput",
          "RefreshControl",
          "ActivityIndicator",
          "Modal",
        ].map(
          (name) => [name, name],
        ),
      ),
      StyleSheet: {
        create: (styles: unknown) => styles,
        hairlineWidth: 0.5,
      },
      Platform: { OS: "ios" },
      ActionSheetIOS: {
        showActionSheetWithOptions: (
          config: { options: string[] },
          pick: (index: number) => void,
        ) => sheets.push({ options: config.options, pick }),
      },
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
    "../display/ChatPreview": { ChatPreview: "ChatPreview" },
    "./AccountDeletionSheet": { AccountDeletionSheet: "AccountDeletionSheet" },
    "../display/ChatDisplaySettings": {
      ChatDisplaySettings: "ChatDisplaySettings",
    },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "lucide-react-native": lucide,
    "../../components/DynamicTypeText": dynamicText,
    "../DynamicTypeText": dynamicText,
    "../../components/ScreenContainer": screenContainer,
    "../ScreenContainer": screenContainer,
    "../../components/settings/SettingsList": settingsList,
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
    "../../theme/ThemeContext": theme,
    "../../theme/flowStyles": { createFlowStyles: () => ({}) },
  });
  const load = (path: string) =>
    harness.load(fileURLToPath(new URL(path, import.meta.url)));
  Object.assign(dynamicText, load("../../components/DynamicTypeText.tsx"));
  Object.assign(
    settingsList,
    load("../../components/settings/SettingsList.tsx"),
  );
  const module = load(file) as Record<string, (props: any) => Element>;
  const Screen = module[exportName]!;
  harness.mount(() =>
    renderRows(
      Screen({
        navigation: { push: (...args: unknown[]) => pushes.push(args) },
        route: params ? { params } : undefined,
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
  /** The pressable or plain container of a settings row, found by its label. */
  const row = (label: string) => {
    let node: Element = text(label);
    for (let depth = 0; depth < 6; depth += 1) {
      node = parent(node);
      if (style(node).minHeight === 52) return node;
    }
    assert.fail(`no settings row for ${label}`);
  };
  return {
    module,
    all,
    text,
    parent,
    row,
    mutations,
    sheets,
    pushes,
    preferences,
    harness,
    render: (node: Element) => renderRows(node, dimensions.fontScale),
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
