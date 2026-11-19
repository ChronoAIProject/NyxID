import { translate, type TranslationValues } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
import assert from "node:assert/strict";
import { ApiError } from "../../lib/api/ApiError";
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
  "NyxBotPushSwitches",
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
    voiceEnabled = false,
    watchAvailable = false,
    watchLink,
    loginCodes,
    nyxbotNotifications,
    params,
    language = "en",
    diagnosticsOffered = false,
    diagnosticsAvailable = false,
    accountPolicy,
    devicePolicy,
    deviceEnabled = false,
    railWindow = false,
    signOut = async () => {},
  }: {
    signOut?: () => Promise<void>;
    connectedTelegram?: boolean;
    pushDevices?: number;
    voiceEnabled?: boolean;
    /** iOS build with the Apple Watch companion module. */
    watchAvailable?: boolean;
    /** The native Apple Watch link the screen talks to. */
    watchLink?: Record<string, unknown>;
    /** `/auth/login-code` calls made by Apple Watch setup. */
    loginCodes?: Record<string, unknown>;
    /** NyxBot notification flag and its saved push switches. */
    nyxbotNotifications?: {
      enabled: boolean;
      push?: Record<string, boolean>;
      pending?: boolean;
    };
    params?: Record<string, unknown>;
    language?: Language;
    diagnosticsOffered?: boolean;
    diagnosticsAvailable?: boolean;
    accountPolicy?: string;
    devicePolicy?: 1|2;
    deviceEnabled?: boolean;
    /** The window behind the Settings sheet shows the Duo's quick rail. */
    railWindow?: boolean;
  } = {},
) {
  let dimensions = { width: 402, height: 874, fontScale: 1, scale: 3 };
  let pending = false;
  const mutations: unknown[] = [];
  const mutationOptions: Record<string, any>[] = [];
  const analyticsSaves:unknown[][]=[];
  const deviceChoices: boolean[] = [];
  const pushes: unknown[][] = [];
  const preferences: unknown[] = [];
  const sheets: Array<{ options: string[]; pick: (index: number) => void }> =
    [];
  const alerts: Array<{
    title: string;
    message?: string;
    buttons?: Array<{ text: string; style?: string; onPress?: () => void }>;
  }> = [];
  const jsx = (type: unknown, props: Element["props"], key?: number) => ({
    type,
    props,
    key,
  });
  const dynamicText = {};
  const screenContainer = { ScreenContainer: "ScreenContainer" };
  const settingsList = {};
  // The extracted switches render through the harness so their rows are real.
  const pushSwitches = {};
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
  const pushPreferenceMutations: unknown[] = [];
  const harness = hookHarness<Element>({
    "../../components/workspace/WorkspaceLayoutContext": {
      WorkspaceRailWindowContext: { value: railWindow },
    },
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => ({
        language,
        t: (phrase: string, values?: TranslationValues) =>
          translate(language, phrase, values),
      }),
    },
    "../notifications/useNotifications": {
      useNotificationsEnabled: () => nyxbotNotifications?.enabled ?? false,
      useNotificationPushPreferences: () => ({
        data: nyxbotNotifications?.pending
          ? undefined
          : {
              automation_result: false,
              automation_problem: false,
              needs_decision: false,
              agent_update: false,
              chat_update: false,
              group_mention: false,
              ...(nyxbotNotifications?.push ?? {}),
            },
        isPending: nyxbotNotifications?.pending ?? false,
      }),
      useUpdateNotificationPushPreferences: () => ({
        mutate: (value: unknown) => pushPreferenceMutations.push(value),
        isPending: false,
      }),
    },
    "./NyxBotPushSwitches": pushSwitches,
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
        ].map((name) => [name, name]),
      ),
      StyleSheet: {
        create: (styles: unknown) => styles,
        hairlineWidth: 0.5,
      },
      Platform: { OS: "ios" },
      Alert: {
        alert: (
          title: string,
          message?: string,
          buttons?: Array<{ text: string; style?: string; onPress?: () => void }>,
        ) => alerts.push({ title, message, buttons }),
      },
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
      useQueryClient: () => ({ clear() {} }),
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        isFetching: false,
        isError: false,
        data:
          queryKey[1] === "voice-options" || queryKey[1] === "voice-settings"
            ? undefined
            : queryKey[1] === "profile"
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
                : queryKey[1] === "voice-features"
                  ? voiceEnabled
                    ? ["assistant:voice"]
                    : []
                  : { enabled: true, policy_version: accountPolicy, diagnostics_offered: diagnosticsOffered, diagnostics_available: diagnosticsAvailable, current_policy_version: diagnosticsOffered ? "2" : "1" },
      }),
      useMutation: (options: Record<string, any>) => {
        mutationOptions.push(options);
        return {
          mutate: (value: unknown) => mutations.push(value),
          isPending: pending,
        };
      },
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
    "../workspace/ScrollEdgeFades": { PageScrollView: "ScrollView" },
    "../ScreenContainer": screenContainer,
    "../../components/settings/SettingsList": settingsList,
    "../../components/SegmentControl": { SegmentControl: renderSegmentControl },
    "../../components/OfflineBanner": { OfflineBanner: "OfflineBanner" },
    "../../components/workspace/WorkspaceUI": { Notice: "Notice" },
    "../../components/Skeleton": {
      SkeletonRows: "SkeletonRows",
      SkeletonCards: "SkeletonCards",
    },
    "../../components/TelegramLinkModal": {
      TelegramLinkModal: "TelegramLinkModal",
    },
    "../../components/ToastOverlay": { ToastOverlay: "ToastOverlay" },
    "../auth/AuthSessionContext": { useAuthSession: () => ({ signOut }) },
    "../../lib/consent": {
      useMobileConsent: () => ({
        accountId: "account",
        enabled: deviceEnabled, policy:devicePolicy,
        setConsent: (value: boolean) => deviceChoices.push(value),
      }),
    },
    "../../lib/analyticsConsentState": {analyticsConsentState:{getSnapshot:()=>({accountId:"account",enabled:false})}},
    "../../lib/analyticsRelay": {saveAnalyticsPermission:(...args:unknown[])=>{analyticsSaves.push(args);}},
    "../../lib/api/http": {
      requestJson: async () => ({ enabled: true }),
      requestBoundAuthenticatedResponse: async () => ({ response: { body: null }, assertOwner: async () => {} }),
    },
    "../../lib/api/ApiError": { ApiError },
    "expo-file-system": {
      Directory: class Directory { list() { return []; } },
      File: class File {
        uri = "cache/export.json";
        open() { return { writeBytes() {}, close() {} }; }
        exists = false;
        create() {}
        write() {}
        delete() {}
      },
      Paths: { cache: "cache" },
    },
    "./accountExport": { cleanStaleAccountExports: () => {}, shareAccountExport: async () => {} },
    "expo-sharing": {
      isAvailableAsync: async () => true,
      shareAsync: async () => {},
    },
    "../../lib/telemetry": { capture() {} },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../lib/api/mobileApi": { mobileApi: {} },
    "../assistant/voice/DictationLanguagePicker": {
      DictationLanguagePicker: "DictationLanguagePicker",
    },
    "../assistant/voice/useDictationLanguage": {
      AUTOMATIC_DICTATION_LANGUAGE: "automatic",
      useDictationLanguage: () => ({
        value: "automatic",
        label: "English (United States)",
        options: [],
        choose() {},
      }),
    },
    "../../lib/api/assistantVoiceApi": {
      nativeVoiceEnvironment: () => {
        throw new Error("Unavailable in this fixture");
      },
      assistantVoiceFeatures: async () => [],
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
    "../../lib/watch/watchLink": {
      nativeWatchLink: () => watchLink ?? (watchAvailable ? {} : null),
    },
    "../../lib/api/loginCodeApi": { loginCodeApi: loginCodes ?? {} },
    "../../lib/api/baseUrl": {
      getApiBaseUrl: () => "https://auth.example.test/api/v1",
    },
    "../../lib/env": { getFrontendUrl: () => "https://app.example.test" },
    "../../lib/auth/authAccount": { getAuthAccountId: () => "account" },
    "../auth/useMfaStepUp": {
      useMfaStepUp: () => ({
        handle: async () => false,
        prompt: null,
        busy: false,
        reset() {},
      }),
    },
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
  Object.assign(pushSwitches, load("./NyxBotPushSwitches.tsx"));
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
    // A group may share its row's label ("Crash diagnostics"): use the first
    // label that sits inside a row.
    text(label);
    const labels = all().filter(
      (item) =>
        item.type === "Text" &&
        [item.props.children].flat(Infinity).join("") === label,
    );
    for (const found of labels) {
      let node: Element | undefined = found;
      for (let depth = 0; depth < 6 && node; depth += 1) {
        node = parent(node);
        if (node && style(node).minHeight === 52) return node;
      }
    }
    assert.fail(`no settings row for ${label}`);
  };
  return {
    module,
    alerts,
    all,
    text,
    parent,
    row,
    mutations,
    mutationOptions, analyticsSaves,
    pushPreferenceMutations,
    deviceChoices,
    sheets,
    pushes,
    preferences,
    harness,
    render: (node: Element) => renderRows(node, dimensions.fontScale),
    language: (value: Language) => {
      language = value;
      harness.render();
    },
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
