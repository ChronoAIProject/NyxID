import type { AssistantTurnPerformance, ChatTurnTimings } from "./assistantTurnPerformance";
import { buildApiError } from "../../lib/api/responseError";
import {
  createAssistantDraftStore,
  assistantDraftKey,
} from "./assistantDraftStore";
import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { AssistantParams } from "../../app/navigationTypes";
import { workspaceNavigateAction } from "../../app/navigationHierarchy";
import type { AssistantUploadedMessage } from "./AssistantUploadSession";
import * as voiceSelection from "./voice/voiceSelection";
import {
  acknowledgementSchema,
  messageSchema,
  historySchema,
  type AssistantAcknowledgement,
  type AssistantMessage,
} from "./assistantTypes";
import { voiceSnapshotSchema } from "./voice/voiceContracts";
import voiceGroupFixtures from "../../../../shared/voice-message-groups.json";
import { VOICE_RECOVERY_TIMEOUT_MS } from "./voice/voiceRequestDeadline";

// Exercise the screen's real handlers and render conditions. Native UI, network,
// and existing assistant/upload hooks are boundaries, not reimplementations of
// the voice entry flow under test.
type Element = { type: unknown; props: Record<string, any>; key?: unknown };
const A = `nyxa-${"a".repeat(32)}`;
const B = `nyxa-${"b".repeat(32)}`;

function expand(element: Element): Element {
  if (typeof element.type === "function")
    return expand((element.type as (props: unknown) => Element)(element.props));
  return {
    ...element,
    props: {
      ...element.props,
      children: [element.props.children]
        .flat(Infinity)
        .map((child) =>
          child && typeof child === "object" && "props" in child
            ? expand(child)
            : child,
        ),
    },
  };
}
function elements(element: Element): Element[] {
  return [
    element,
    ...element.props.children
      .flat(Infinity)
      .flatMap((child: unknown) =>
        child && typeof child === "object" && "props" in child
          ? elements(child as Element)
          : [],
      ),
  ];
}

// The shared composer controls are exercised through their accessible button contract.
function composerButton(props: {
  kind?: string;
  label: string;
  disabled?: boolean;
  onPress: () => void;
}): Element {
  return {
    type: "Pressable",
    props: {
      kind: props.kind,
      accessibilityLabel: props.label,
      disabled: props.disabled,
      onPress: props.onPress,
      children: [],
    },
  };
}

// Exercise enabled and unavailable server/runtime admission separately.
function setup(
  conversationId?: string,
  enableVoiceForTest = true,
  platform = "ios",
  reducedMotion = false,
  translations: Record<string, string> = {},
) {
  let onAdopt: ((id: string) => void) | undefined;
  let headerOptions: Record<string, any> = {};
  let columnHeader: Record<string, any> | undefined;
  const headerModule = {};
  let navigationDisabled = false;
  const draft = deferred<string>();
  const events: unknown[][] = [];
  const perf = { enabled: false, sample: undefined as AssistantTurnPerformance | undefined, submitted: [] as ChatTurnTimings[] };
  const nativePerf = {
    frames: [] as { focused: boolean; streaming: boolean; scope: unknown }[],
    events: [] as string[],
  };
  const alerts: { text: string; style?: string; onPress?: () => void }[][] = [];
  const drafts: {
    agentId: string;
    signal: AbortSignal;
    response: ReturnType<typeof deferred<string>>;
  }[] = [];
  const featureQueries: { enabled: boolean }[] = [];
  let epoch = 7;
  // Ordinary iPhone width: below the persistent-sidebar breakpoint.
  let windowWidth = 390;
  let bellEnabled = false;
  let fontScale = 1;
  // A nonzero right inset stands in for the folded Duo's corner cluster.
  let rightInset = 0;
  // The folded Duo's quick rail is showing (it owns New chat and Scan QR).
  let quickRail = false;
  let dictating = false;
  let focused = true;
  let keyboardVisible = false;
  let hideOnDismiss = false;
  const keyboardListeners = new Set<() => void>();
  const hideKeyboard = () => {
    keyboardVisible = false;
    for (const listener of [...keyboardListeners]) listener();
  };
  let transcriptFollowing = true;
  let transcriptReady = false;
  let features: string[] | undefined = ["assistant:voice"];
  const agent = {
    id: "agent-a",
    name: "nyxbot",
    display_name: "NyxBot",
    can_use: true,
    kind: "nyxbot",
    status: "idle",
    model: "nyxagent/chat",
  };
  function history(id: string) {
    return {
      conversation: {
        id,
        title: "Saved chat",
        model: "nyxagent/chat",
        agent,
        active_turn: null,
        last_message_at: "2026-10-28T01:00:00Z",
        message_count: 12,
      } as Record<string, any>,
      messages: [] as AssistantMessage[],
      waiting: [],
      acknowledgements: [] as AssistantAcknowledgement[],
      approvals: [] as ReturnType<typeof historySchema.parse>["approvals"],
    };
  }
  const assistant = {
    agent,
    agentId: agent.id,
    conversationId,
    history: conversationId ? history(conversationId) : undefined,
    models: [],
    model: "nyxagent/chat",
    agents: [agent],
    agentsError: undefined as string | undefined,
    reloadAgents: async () => {
      events.push(["reloadAgents"]);
      assistant.agentsError = undefined;
    },
    conversations: [],
    inputAttachments: [],
    online: true,
    sending: false,
    loading: false,
    loadingOlder: false,
    recoveryRequired: false,
    error: undefined as string | undefined,
    mutating: false,
    openConversation: (id?: string, ack?: string) => {
      events.push(["open", id, ack]);
      assistant.conversationId = id;
      assistant.history = id ? history(id) : undefined;
    },
    recover: async () => {
      events.push(["recover"]);
    },
    send: async (
      text: string,
      _options?: unknown,
      files?: AssistantUploadedMessage,
      _acknowledgement?: string,
      model?: string,
      sample?: AssistantTurnPerformance,
    ) => {
      perf.sample = sample;
      events.push(model ? ["send", text, files, model] : ["send", text, files]);
      return true;
    },
    setModel: (model: string) => events.push(["model", model]),
    stop: async () => {
      events.push(["stop"]);
    },
    decideApproval: async (id: string, approved: boolean) => { events.push(["decideApproval", id, approved]); },
  };
  const guidance = {
    canSteer: false,
    submitting: false,
    notice: undefined as any,
    maxChars: 32000,
    submit: async (text: string, count: number) => {
      events.push(["guidance", text, count]);
      return true;
    },
    sendAsMessage: async (_send: any) => {},
  };
  const connection = {
    connection: undefined as any,
    unavailable: false,
    choose: () => events.push(["choose-connection"]),
    prefix: (text: string) => text,
    sent: () => events.push(["connection-sent"]),
  };
  const uploads = {
    sourcePicker: undefined as undefined | { choices: { label: string; onPress: () => void }[]; onClose: () => void },
    files: [] as any[],
    session: {
      sending: false,
      model: undefined as string | undefined,
      ensureConversation: (create: () => Promise<string>) => create(),
      applyDraftModel: async (id: string, _change: unknown) => {
        events.push(["patch-draft-model", id, uploads.session.model]);
      },
      consumeDraft: async () => {},
      submit: async (
        submit: (
          payload: AssistantUploadedMessage | undefined,
        ) => Promise<unknown>,
      ) => submit(undefined),
    },
    blocked: false,
    picking: false,
    pick: async () => {},
    choose: (extra?: any) => events.push(["choose-attachment", extra]),
    remove: (key: string) => {
      uploads.files = uploads.files.filter((file) => file.key !== key);
    },
  };
  const props = {
    contextual: false,
    route: {
      params: {
        home: !conversationId,
        agentId: agent.id,
        conversationId,
      } as AssistantParams,
    },
  };
  const mocks = {
    "../../lib/performance/metrics": {
      beginPerformanceSample: () => perf.enabled ? epoch : null,
      submitPerformanceSample: (_epoch: number, _event: string, timings: ChatTurnTimings) => perf.submitted.push(timings),
    },
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => ({
        t: (phrase: string) => translations[phrase] ?? phrase,
      }),
    },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => reducedMotion,
    },
    "./assistantDraftStore": {
      assistantDraftKey,
      assistantDraftStore: createAssistantDraftStore(() => epoch),
    },
    "./ChatIdentityHeader": headerModule,
    "react-native-safe-area-context": {
      SafeAreaView: "SafeAreaView",
      useSafeAreaInsets: () => ({
        top: 0,
        left: 0,
        bottom: 0,
        right: rightInset,
      }),
    },
    "../../components/workspace/WorkspaceLayoutContext": {
      WorkspaceCornerSensorContext: {
        get value() {
          return rightInset > 0;
        },
      },
      WorkspaceQuickRailContext: {
        get value() {
          return quickRail;
        },
      },
    },
    "../display/ChatLayoutContext": {
      useComposerHeight: () => 120,
      useChatColumn: () => ({
        width: "100%",
        maxWidth: 758,
        alignSelf: "center",
      }),
    },
    "../../components/workspace/ScrollEdgeFades": {
      ScrollEdgeFades: "ScrollEdgeFades",
      useScrollEdges: () => ({
        edges: { start: false, end: false },
        layout: (height: number) => events.push(["fade-layout", height]),
        content: (height: number) => events.push(["fade-content", height]),
        scroll: (offset: number) => events.push(["fade-scroll", offset]),
      }),
    },
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key?: unknown) => ({
        type,
        props,
        key,
      }),
    },
    "react-native": {
      useWindowDimensions: () => ({
        width: windowWidth,
        height: 844,
        fontScale,
      }),
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "FlatList",
          "Modal",
          "Pressable",
          "ScrollView",
          "Text",
          "View",
        ].map((name) => [name, name]),
      ),
      Platform: { OS: platform },
      Alert: {
        alert: (
          _title: string,
          _message: unknown,
          actions: (typeof alerts)[number],
        ) => alerts.push(actions),
      },
      LayoutAnimation: { configureNext() {}, Presets: { easeInEaseOut: {} } },
      Keyboard: {
        isVisible: () => keyboardVisible,
        addListener: (event: string, listener: () => void) => {
          assert.equal(event, "keyboardDidHide");
          keyboardListeners.add(listener);
          return { remove: () => keyboardListeners.delete(listener) };
        },
        dismiss: () => {
          events.push(["dismissKeyboard"]);
          if (hideOnDismiss) hideKeyboard();
        },
      },
      Animated: {
        View: "Animated.View",
        Value: class {
          setValue() {}
          interpolate(value: unknown) { return value; }
        },
        timing: () => ({ start: (complete: any) => complete({ finished: true }), stop() {} }),
      },
      Easing: { out: (value: unknown) => value, cubic: "cubic" },
      StyleSheet: { create: (value: unknown) => value, hairlineWidth: 0.5 },
    },
    "@react-navigation/native": {
      useIsFocused: () => focused,
      useNavigation: () => ({
        setParams: (params: Record<string, unknown>) => {
          events.push(["params", { ...params }]);
          Object.assign(props.route.params, params);
        },
        dispatch: (action: unknown) => events.push(["dispatch", action]),
        push: (name: string, params: unknown) =>
          events.push(["push", name, params]),
        navigate: (name: string, params: unknown) =>
          events.push(["navigate", name, params]),
        goBack: () => {},
      }),
    },
    "@tanstack/react-query": {
      useQuery: (options: { enabled: boolean }) => {
        featureQueries.push(options);
        return { data: features };
      },
    },
    "expo-web-browser": { openBrowserAsync: async () => {} },
    "./AssistantPendingState": {
      AssistantApprovalCard: "ApprovalCard",
      AssistantWaitingNote: "WaitingNote",
    },
    "./AssistantSystemMessage": { AssistantSystemMessage: "SystemMessage" },
    "./agentMood": {
      moodInputFor: () => ({ activeTurn: null }),
      settledReplyOf: () => null,
      useAgentMood: () => "idle",
    },
    "../../lib/avatars/useOwnNyxBot": {
      useOwnNyxBotIdentity: () => ({ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }),
    },
    "./AssistantReplyBody": {
      AssistantReplyBody: (props: any) => ({
        type: "ReplyBody",
        props: { ...props, children: props.activity },
      }),
    },
    "./AssistantComposerSettings": {
      AssistantComposerSettings: "ComposerSettings",
      useComposerAccessMode: () => "nyxmode",
    },
    "react-native-markdown-display": {
      default: "Markdown",
      MarkdownIt: () => ({}),
      __esModule: true,
    },
    "lucide-react-native": Object.fromEntries(
      [
        "ChevronDown",
        "Check",
        "MoreHorizontal",
        "Paperclip",
        "ScanQrCode",
        "SquarePen",
        "ArrowUp",
        "Square",
        "X",
        "Mic",
      ].map((name) => [name, name]),
    ),
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (input: {
        options: Record<string, unknown>;
        disabled: boolean;
      }) => {
        navigationDisabled = input.disabled;
        headerOptions = input.options;
      },
      useWorkspaceColumnHeader: (input: Record<string, any>) => {
        columnHeader = input;
      },
      WorkspaceHeaderVisibility: "WorkspaceHeaderVisibility",
    },
    "../../components/workspace/WorkspaceSheetSurface": {
      WorkspaceSheetSurface: "WorkspaceSheetSurface",
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["WorkspacePage", "ActionButton", "Card", "Field", "Label", "Notice"].map(
        (name) => [name, name],
      ),
    ),
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { warningTone: {} }, mode: "light" }),
    },
    "../../components/workspace/WorkspaceAgentPicker": {
      WorkspaceAgentPicker: (props: Record<string, any>) => ({
        type: "WorkspaceAgentPicker",
        props: { ...props, children: [props.scrollHeader, props.scrollFooter] },
      }),
    },
    "./useAssistantGuidance": { useAssistantGuidance: () => guidance },
    "./useComposerConnection": { useComposerConnection: () => connection },
    "./ComposerConnectionPicker": {
      ComposerConnectionPicker: "ConnectionPicker",
    },
    "../../components/workspace/WorkspaceRightPanel": {
      RightPanelProvider: "RightPanelProvider",
      RightPanelRegistration: "RightPanelRegistration",
      RightPanelSlot: "RightPanelSlot",
    },
    "./NyxBotHome": { NyxBotHome: "NyxBotHome" },
    "./AssistantRightPanel": { AssistantRightPanel: "AssistantRightPanel" },
    "./useAssistant": {
      useAssistant: (_id: unknown, _agent: unknown, adopt: typeof onAdopt) => {
        onAdopt = adopt;
        return assistant;
      },
    },
    "./AttachmentSourcePicker": { AttachmentSourcePicker: "AttachmentSourcePicker" },
    "./useAssistantUploads": { useAssistantUploads: () => uploads },
    "../../lib/api/assistantApi": { assistantApi: { changeModel: () => {} } },
    "./useComposerDictation": {
      useComposerDictation: () =>
        dictating
          ? { active: true, phase: "listening" }
          : { active: false, phase: "idle" },
      appendDictation: (draft: string, text: string) =>
        `${draft}${draft ? " " : ""}${text}`,
    },
    "./ComposerDictation": {
      ComposerDictationButton: () =>
        composerButton({ label: "Dictate message", onPress: () => {} }),
      ComposerDictationReview: "ComposerDictationReview",
    },
    "./useTranscriptScrollFollow": {
      useTranscriptScrollFollow: () => ({
        following: transcriptFollowing,
        ready: transcriptReady,
        listProps: {
          onLayout: (event: any) =>
            events.push(["follow-layout", event.nativeEvent.layout.height]),
          onContentSizeChange: (_width: number, height: number) =>
            events.push(["follow-content", height]),
          onScroll: (event: any) =>
            events.push(["follow-scroll", event.nativeEvent.contentOffset.y]),
          onScrollBeginDrag: () => events.push(["follow-drag"]),
          onScrollEndDrag: () => events.push(["follow-drag-end"]),
          onMomentumScrollBegin: () => events.push(["follow-momentum"]),
          onMomentumScrollEnd: () => events.push(["follow-momentum-end"]),
        },
        follow: () => {},
        pause: () => {},
      }),
    },
    "../../lib/performance/useNativePerformance": {
      useChatNativeFrames: (options: { focused: boolean; streaming: boolean; scope: unknown }) => {
        nativePerf.frames.push(options);
        return {
          beginDrag: () => nativePerf.events.push("drag"),
          endDrag: () => nativePerf.events.push("drag-end"),
          momentumBegin: () => nativePerf.events.push("momentum"),
          momentumEnd: () => nativePerf.events.push("momentum-end"),
        };
      },
    },
    "./AssistantAcknowledgementCard": {
      AssistantAcknowledgementCard: "AssistantAcknowledgementCard",
    },
    "./AssistantCmaOperation": {
      AssistantCmaOperation: "AssistantCmaOperation",
    },
    "./AssistantTurnStatus": { AssistantTurnStatus: "AssistantTurnStatus" },
    "./AssistantMessageNotices": {
      AssistantMessageNotices: "AssistantMessageNotices",
      QuietButton: "QuietButton",
    },
    "./AssistantActivityDisclosure": {
      AssistantActivityDisclosure: "AssistantActivityDisclosure",
    },
    "../machines/MachineToolCard": { MachineToolCard: "MachineToolCard" },
    "./AssistantAcknowledgements": {
      AssistantAcknowledgements: "AssistantAcknowledgements",
    },
    "./AssistantMessageFooter": {
      AssistantMessageFooter: "AssistantMessageFooter",
    },
    "./AssistantWorkingContext": {
      AssistantWorkingContext: "AssistantWorkingContext",
    },
    "./AssistantBanner": {
      AssistantBanner: "AssistantBanner",
      AssistantConnectionBanner: "AssistantConnectionBanner",
      AssistantErrorNotice: "AssistantErrorNotice",
      JumpToLatest: "JumpToLatest",
    },
    "./AssistantCopyButton": { AssistantCopyButton: "AssistantCopyButton" },
    "./AssistantAttachmentView": {
      AssistantAttachmentView: "AssistantAttachmentView",
    },
    "./AssistantPendingUploads": {
      AssistantPendingUploads: "AssistantPendingUploads",
    },
    "./AssistantMentionInput": {
      AssistantMentionInput: (props: {
        renderActions?: (mention: null, layout: {
          compact: boolean;
          onLeadingWidth: (width: number) => void;
          onTrailingWidth: (width: number) => void;
        }) => unknown;
      }) => ({
        type: "AssistantMentionInput",
        props: {
          ...props,
          children: props.renderActions?.(null, {
            compact: false,
            onLeadingWidth() {},
            onTrailingWidth() {},
          }),
        },
      }),
    },
    "./ChatComposer": {
      ChatComposerFrame: ({ children }: { children: unknown }) => ({
        type: "ChatComposerFrame",
        props: { children },
      }),
      ChatComposerRow: ({ children }: { children: unknown }) => ({
        type: "ChatComposerRow",
        props: { children },
      }),
      ComposerAttachButton: composerButton,
      ComposerSubmitButton: composerButton,
    },
    "./assistantSelectableMarkdown": { assistantSelectableMarkdown: {} },
    "../plugins/NativeAuthoredSkillReview": {
      NativeAuthoredSkillReview: "NativeAuthoredSkillReview",
    },
    "../../lib/env": {
      getFrontendUrl: () => "https://nyx.example",
      NYXID_TRUSTED_FRONTEND_URLS: ["https://nyx.example"],
      firstPartyFrontendUrls: () => ["https://nyx.example"],
    },
    "./voice/AssistantVoiceConversation": {
      AssistantVoiceConversation: "AssistantVoiceConversation",
    },
    "./voice/voiceSelection": {
      ...voiceSelection,
      NATIVE_VOICE_ENTRY_ENABLED: enableVoiceForTest,
    },
    "../../lib/api/assistantVoiceApi": {
      assistantVoiceFeatures: async () => features,
    },
    "../../lib/api/assistantUploadsApi": {
      assistantUploadsApi: {
        createDraft: (agentId: string, signal: AbortSignal) => {
          const response = drafts.length === 0 ? draft : deferred<string>();
          drafts.push({ agentId, signal, response });
          return response.promise;
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../notifications/useNotifications": {
      useNotificationIndicator: () => ({
        enabled: bellEnabled,
        live: false,
        unread: 0,
        needsDecision: 0,
        badge: "",
      }),
      useNotificationRuntime: () => ({ active: true, online: true }),
      useVisibleThreadRead: () => {},
    },
  };
  const harness = hookHarness<Element>(mocks);
  // React Navigation mounts headerRight separately from the screen.
  const headerHarness = hookHarness<Element>({ ...mocks });
  headerHarness.mock("./ChatHeaderMenu", headerHarness.load(
    fileURLToPath(new URL("./ChatHeaderMenu.tsx", import.meta.url)),
  ));
  harness.mock("expo-linear-gradient", { LinearGradient: "LinearGradient" });
  Object.assign(
    headerModule,
    harness.load(
      fileURLToPath(new URL("./ChatIdentityHeader.tsx", import.meta.url)),
    ),
  );
  harness.mock(
    "./ChatComposerInput",
    harness.load(
      fileURLToPath(new URL("./ChatComposerInput.tsx", import.meta.url)),
    ),
  );
  harness.mock("./TranscriptViewport", {
    TranscriptViewport: ({ children, onTouchStart, viewportRef }: any) => ({
      type: "View",
      props: { style: { flex: 1 }, onTouchStart, ref: viewportRef, children },
    }),
  });
  harness.mock(
    "./ChatSurface",
    harness.load(fileURLToPath(new URL("./ChatSurface.tsx", import.meta.url))),
  );
  harness.mock(
    "./ChatHeaderActions",
    headerHarness.load(
      fileURLToPath(new URL("./ChatHeaderActions.tsx", import.meta.url)),
    ),
  );
  const Screen = harness.load(
    fileURLToPath(new URL("./AssistantScreen.tsx", import.meta.url)),
  ).AssistantScreen as (input: typeof props) => Element;
  harness.mount(() => expand(Screen(props)));
  headerHarness.mount(() => {
    const tree = expand(headerOptions.headerRight());
    const anchor = elements(tree).find(
      (item) => item.props.testID === "chat-header-actions",
    )!;
    anchor.props.ref.current = {
      measureInWindow: (report: (...values: number[]) => void) =>
        report(windowWidth - 100, 66, 88, 44),
    };
    return tree;
  });
  const headerRight = () => headerHarness.render();
  const headerNodes = () => elements(headerRight());
  const menuItems = () =>
    headerNodes().filter((n) => n.props.accessibilityRole === "menuitem");
  const finishMenuDismissal = () => {
    headerHarness.flush();
    if (platform === "ios")
      headerNodes()
        .find((n) => n.type === "Modal")!
        .props.onDismiss();
    harness.flush();
  };
  function options() {
    headerNodes()
      .find((n) => n.props.accessibilityLabel === "Chat options")!
      .props.onPress();
    headerHarness.flush();
  }
  const all = () => elements(harness.value());
  const button = (name: string) =>
    all().find((element) => element.props.accessibilityLabel === name);
  const voice = () =>
    all().find((element) => element.type === "AssistantVoiceConversation");
  return {
    harness,
    perf,
    nativePerf,
    navigationDisabled: () => navigationDisabled,
    assistant,
    uploads,
    connection,
    guidance,
    props,
    events,
    alerts,
    drafts,
    featureQueries,
    draft,
    adopt: (id: string) => {
      assistant.conversationId = id;
      onAdopt?.(id);
      harness.render();
    },
    all,
    button,
    columnHeader: () => columnHeader!,
    header: () => expand(headerOptions.headerTitle()),
    headerRight,
    headerActions: () => {
      const nodes = headerNodes();
      return nodes.some((n) => n.type === "Modal" && n.props.visible) ? [] : nodes.filter((n) => n.type === "FloatingItem");
    },
    options,
    menuItems,
    finishMenuDismissal,
    chooseOption: (label: string) => {
      menuItems().find((item) => item.props.accessibilityLabel === label)!.props.onPress();
      finishMenuDismissal();
    },
    setWidth: (value: number) => {
      windowWidth = value;
      harness.render();
    },
    setBell: (value: boolean) => {
      bellEnabled = value;
      harness.render();
    },
    setFontScale: (value: number) => {
      fontScale = value;
      harness.render();
    },
    setDictating: (value: boolean) => {
      dictating = value;
      harness.render();
    },
    setQuickRail: (value: boolean) => {
      quickRail = value;
      harness.render();
    },
    setCornerSensor: (inset: number) => {
      rightInset = inset;
      harness.render();
    },
    compose: () => {
      const actions = headerRight();
      elements(actions)
        .find((n) => n.props.accessibilityLabel === "New chat")!
        .props.onPress();
      harness.flush();
    },
    chooseAgent: () => {
      const header = expand(headerOptions.headerTitle());
      elements(header)
        .find((element) =>
          element.props.accessibilityLabel?.endsWith("Choose agent"),
        )!
        .props.onPress();
      harness.flush();
    },
    voice,
    input: () =>
      all().find((element) => element.type === "AssistantMentionInput")!,
    setEpoch: (value: number) => {
      epoch = value;
    },
    setFocused: (value: boolean) => {
      focused = value;
      harness.render();
    },
    keyboard: {
      show: () => {
        keyboardVisible = true;
      },
      hide: hideKeyboard,
      hideOnDismiss: () => {
        hideOnDismiss = true;
      },
      listeners: () => keyboardListeners.size,
    },
    pauseTranscript: () => {
      transcriptFollowing = false;
      harness.render();
    },
    setTranscriptReady: (value: boolean) => {
      transcriptReady = value;
      harness.render();
    },
    setFeatures: (value: string[] | undefined) => {
      features = value;
      harness.render();
    },
  };
}

function liveVoice(text: string, sealed = false) {
  return voiceSnapshotSchema.parse({
    type: "snapshot",
    session: {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      created_at: "2026-11-09T01:00:00Z",
      closed_at: null,
      generation: 1,
      state: "active",
      muted: false,
      input_muted: false,
      end_requested: false,
      control_revision: 1,
      observed_seconds: 2,
      reserved_until: 30,
      final_usage_confirmed: false,
      end_reason: null,
      idle_warning: false,
    },
    captions: [
      { id: "live-user", speaker: "user", text, sealed, complete: sealed },
    ],
    tasks: [],
  });
}

test("a shielded voice link publication renders as completed without pending UI", () => {
  const x = setup(A, false);
  const link = "https://nyxid.ai/connect/bot/nyx_bcl_example";
  const message = messageSchema.parse({
    id: "published-link",
    seq: 3,
    turn_id: "hidden-turn",
    role: "assistant",
    text: `<${link}>`,
    status: "completed",
    execution_pending: true,
    via: "voice",
    error_code: null,
    created_at: "2026-11-14T01:00:00Z",
    activities: [],
    attachments: [],
  });
  // The API field is ignored by the schema; even a retained field must not
  // turn an assistant publication into pending execution UI.
  Object.assign(message, { execution_pending: true });
  x.assistant.history!.messages = [message];
  x.harness.render();
  const list = x.all().find((node) => node.type === "FlatList")!;
  assert.equal(list.props.data.length, 1);
  const rendered = elements(
    expand(list.props.renderItem({ item: list.props.data[0], index: 0 })),
  );
  const footer = rendered.find(
    (node) => node.type === "AssistantMessageFooter",
  );
  assert.equal(footer?.props.text, `<${link}>`);
  assert.equal(footer?.props.report.messageId, message.id);
  assert.equal(
    rendered.some((node) =>
      ["ActivityIndicator", "AssistantTurnStatus"].includes(String(node.type)),
    ),
    false,
  );
  assert.doesNotMatch(
    JSON.stringify(rendered),
    /Queued|Waiting for|Thinking|Speaking/,
  );
  x.harness.unmount();
});

test("persisted voice segments render one Copy/date footer and Report targets the latest feedback anchor", () => {
  for (const savedFeedback of [false, true]) {
    const x = setup(A);
    const fixture = voiceGroupFixtures[0]!;
    x.assistant.history!.messages = fixture.rows.map((row, index) =>
      messageSchema.parse({
        ...row,
        seq: index + 6,
        role: "assistant",
        status: "completed",
        error_code: null,
        created_at: `2026-11-09T01:00:0${index}Z`,
        via: "voice",
        ...(savedFeedback && index === 1 ? { feedback: { vote: "down" } } : {}),
      }),
    );
    x.harness.render();
    const list = x.all().find((node) => node.type === "FlatList")!;
    assert.equal(list.props.data.length, 1);
    const rendered = elements(
      expand(list.props.renderItem({ item: list.props.data[0], index: 0 })),
    );
    const footers = rendered.filter(
      (node) => node.type === "AssistantMessageFooter",
    );
    assert.equal(footers.length, 1);
    assert.equal(footers[0]!.props.text, fixture.copy![0]);
    assert.equal(footers[0]!.props.timestamp, "2026-11-09T01:00:00Z");
    assert.equal(footers[0]!.props.report.messageId, savedFeedback ? "7" : "8");
    assert.equal(footers[0]!.props.report.id, x.assistant.conversationId);
    assert.equal(x.assistant.history!.messages.length, 3);
    x.harness.unmount();
  }
});

test("appending voice parts keeps the mobile row key stable and Report follows its anchor", () => {
  const x = setup(A);
  const rows = voiceGroupFixtures[0]!.rows.map((row, index) =>
    messageSchema.parse({
      ...row,
      seq: index + 6,
      role: "assistant",
      status: "completed",
      error_code: null,
      created_at: `2026-11-09T01:00:0${index}Z`,
      via: "voice",
    }),
  );
  x.assistant.history!.messages = rows.slice(0, 1);
  x.harness.render();
  const list = () => x.all().find((node) => node.type === "FlatList")!;
  const key = list().props.keyExtractor(list().props.data[0]);
  x.assistant.history!.messages = rows;
  x.harness.render();
  assert.equal(list().props.keyExtractor(list().props.data[0]), key);
  const rendered = elements(
    expand(list().props.renderItem({ item: list().props.data[0], index: 0 })),
  );
  assert.equal(
    rendered.find((node) => node.type === "AssistantMessageFooter")!.props
      .report.messageId,
    "8",
  );
  x.harness.unmount();
});
test("a navigation target for a middle voice segment reveals its stable group row", () => {
  const x = setup(A);
  x.assistant.history!.messages = voiceGroupFixtures[0]!.rows.map(
    (row, index) =>
      messageSchema.parse({
        ...row,
        seq: index + 6,
        role: "assistant",
        status: "completed",
        error_code: null,
        created_at: "2026-11-09T01:00:00Z",
        via: "voice",
      }),
  );
  x.props.route.params.messageId = "7";
  x.harness.render();
  const list = x.all().find((node) => node.type === "FlatList")!;
  const scrolls: { index: number }[] = [];
  list.props.ref.current = {
    scrollToIndex: (target: { index: number }) => scrolls.push(target),
  };
  x.harness.render();
  assert.equal(list.props.data[0].id, "6");
  assert.deepEqual(
    scrolls.map((target) => target.index),
    [0],
  );
  x.harness.render();
  assert.equal(scrolls.length, 1);
  x.harness.unmount();
});
test("voice footers hide live continuations and keep sealed disconnect partial groups", async () => {
  for (const [sealed, state, interrupted, visible] of [
    [false, "active", false, false],
    [true, "active", false, false],
    [true, "closed", false, true],
    [true, "failed", false, true],
    [true, "active", true, true],
  ] as const) {
    const x = setup(A);
    x.assistant.history!.messages = voiceGroupFixtures[0]!.rows
      .slice(0, 2)
      .map((row, index) =>
        messageSchema.parse({
          ...row,
          seq: index + 6,
          role: "assistant",
          status: "completed",
          error_code: null,
          created_at: `2026-11-09T01:00:0${index}Z`,
          via: "voice",
          voice: {
            ...row.voice,
            sealed: index === 0 || sealed,
            complete: index === 0,
            interrupted: index !== 0 && interrupted,
          },
        }),
      );
    x.button("Open voice call")!.props.onPress();
    await x.harness.settle();
    const snapshot = liveVoice("");
    snapshot.session.id = x.assistant.history!.messages[0]!.voice!.session_id;
    snapshot.session.state = state;
    snapshot.captions = [];
    x.voice()!.props.onSnapshot(snapshot);
    x.harness.flush();
    const list = x.all().find((node) => node.type === "FlatList")!;
    assert.equal(list.props.data.length, 1);
    const rendered = elements(
      expand(list.props.renderItem({ item: list.props.data[0], index: 0 })),
    );
    assert.equal(
      rendered.filter((node) => node.type === "AssistantMessageFooter").length,
      visible ? 1 : 0,
    );
    if (visible) {
      const footer = rendered.find(
        (node) => node.type === "AssistantMessageFooter",
      )!;
      assert.equal(footer.props.text, list.props.data[0].text);
      assert.equal(footer.props.timestamp, "2026-11-09T01:00:00Z");
      assert.equal(footer.props.report.messageId, "7");
    }
    x.harness.unmount();
  }
});

test("live speech grows in the main chat and sealed history replaces it once", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  const list = () => x.all().find((node) => node.type === "FlatList")!;
  const originalKey = list().key;
  x.voice()!.props.onSnapshot(liveVoice("上午"));
  x.harness.flush();
  assert.equal(list().props.data.length, 1);
  assert.equal(list().props.data[0].text, "上午");
  const render = () =>
    elements(
      expand(list().props.renderItem({ item: list().props.data[0], index: 0 })),
    );
  assert.equal(
    render().some((node) => node.type === "AssistantMessageFooter"),
    false,
  );
  x.voice()!.props.onSnapshot(liveVoice("上午写方案"));
  x.harness.flush();
  assert.equal(list().props.data.length, 1);
  assert.equal(list().props.data[0].text, "上午写方案");
  const persisted = messageSchema.parse({
    id: "live-user",
    seq: 1,
    turn_id: "turn",
    role: "user",
    text: "上午写方案。",
    status: "completed",
    error_code: null,
    created_at: "2026-11-09T01:00:00Z",
    via: "voice",
    voice: {
      session_id: liveVoice("").session.id,
      segment_id: "live-user",
      sealed: true,
      complete: true,
    },
  });
  x.assistant.history!.messages = [persisted];
  x.harness.render();
  assert.equal(list().props.data.length, 1);
  assert.equal(list().props.data[0].text, persisted.text);
  assert.ok(render().some((node) => node.type === "AssistantMessageFooter"));
  x.voice()!.props.onClose();
  x.harness.flush();
  assert.equal(list().key, originalKey);
  assert.equal(list().props.data.length, 1);
  x.harness.unmount();
});

test("only a real caption creates a transcription row, which updates in place and never leaves an extra footer", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  const list = () => x.all().find((node) => node.type === "FlatList")!;
  const footerTranscribing = () =>
    elements(expand(list().props.ListFooterComponent)).some((node) =>
      node.props.children.includes("Transcribing…"),
    );
  const renderedRow = (index = 0) =>
    elements(
      expand(
        list().props.renderItem({ item: list().props.data[index], index }),
      ),
    );
  x.voice()!.props.onSnapshot({ ...liveVoice(""), captions: [] });
  x.harness.flush();
  assert.equal(list().props.data.length, 0);
  assert.equal(footerTranscribing(), false);

  x.voice()!.props.onSnapshot(liveVoice(""));
  x.harness.flush();
  assert.equal(list().props.data.length, 1);
  const captionId = list().props.data[0].id;
  assert.equal(
    renderedRow().filter((node) =>
      node.props.children.includes("Transcribing…"),
    ).length,
    1,
  );
  assert.equal(footerTranscribing(), false);

  x.voice()!.props.onSnapshot(liveVoice("正在"));
  x.harness.flush();
  assert.equal(list().props.data[0].id, captionId);
  assert.equal(list().props.data[0].text, "正在");
  x.voice()!.props.onSnapshot(liveVoice("正在说话"));
  x.harness.flush();
  assert.equal(list().props.data.length, 1);
  assert.equal(list().props.data[0].text, "正在说话");
  assert.equal(footerTranscribing(), false);

  const completed = liveVoice("正在说话。", true);
  completed.captions.push({
    id: "live-assistant",
    speaker: "assistant",
    text: "听到了。",
    sealed: true,
    complete: true,
  });
  for (let repeat = 0; repeat < 3; repeat++) {
    x.voice()!.props.onSnapshot(completed);
    x.harness.flush();
    assert.equal(list().props.data.length, 2);
    assert.equal(list().props.data[0].id, captionId);
    assert.equal(footerTranscribing(), false);
    assert.equal(
      renderedRow().some((node) =>
        node.props.children.includes("Transcribing…"),
      ),
      false,
    );
  }

  const next = {
    ...completed,
    captions: [
      ...completed.captions,
      { ...liveVoice("").captions[0], id: "next-user" },
    ],
  };
  x.voice()!.props.onSnapshot(next);
  x.harness.flush();
  assert.equal(list().props.data.length, 3);
  assert.equal(
    renderedRow(2).filter((node) =>
      node.props.children.includes("Transcribing…"),
    ).length,
    1,
  );
  x.voice()!.props.onSnapshot({
    ...next,
    captions: next.captions.map((caption) =>
      caption.id === "next-user" ? { ...caption, text: "下一句" } : caption,
    ),
  });
  x.harness.flush();
  assert.equal(list().props.data.length, 3);
  assert.equal(list().props.data[2].id, "next-user");
  assert.equal(list().props.data[2].text, "下一句");
  x.voice()!.props.onClose();
  x.harness.flush();
  assert.equal(footerTranscribing(), false);
  assert.equal(list().props.data[2].text, "下一句");
  const retained = renderedRow(2);
  assert.ok(
    retained.some((node) => node.props.children.includes("Partial transcript")),
  );
  assert.equal(
    retained.some((node) => node.props.children.includes("Transcribing…")),
    false,
  );
  x.harness.unmount();
});

test("old voice callbacks cannot populate a different chat or account", async () => {
  for (const accountChange of [false, true]) {
    const x = setup(A);
    x.button("Open voice call")!.props.onPress();
    await x.harness.settle();
    const stale = x.voice()!.props;
    if (accountChange) x.setEpoch(8);
    else x.assistant.openConversation(B);
    x.harness.render();
    stale.onSnapshot(liveVoice("old account words"));
    x.harness.flush();
    assert.equal(
      x.all().find((node) => node.type === "FlatList")!.props.data.length,
      0,
    );
    const footer = elements(
      expand(
        x.all().find((node) => node.type === "FlatList")!.props
          .ListFooterComponent,
      ),
    );
    assert.equal(
      footer.some(
        (node) =>
          node.props.accessibilityLabel === "Your speech is being transcribed",
      ),
      false,
    );
    x.harness.unmount();
  }
});

test("the Duo rail opens Details during a call and the call exposes its own preferences", async () => {
  const x = setup(A);
  x.setQuickRail(true);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  x.columnHeader().more.onPress();
  assert.equal(x.alerts.length, 0);
  assert.equal(x.events.at(-1)?.[1], "AssistantDetails");
  // Uncontrolled: the call shows its own Voice preferences control.
  assert.equal(x.voice()!.props.preferencesOpen, undefined);
  x.harness.unmount();
});

test("a wide chat exposes voice preferences in the dropdown for the active call", async () => {
  const x = setup(A);
  x.setWidth(1024);
  x.options();
  assert.deepEqual(
    x.menuItems().map((n) => n.props.accessibilityLabel),
    ["Chat details", "Settings"],
  );
  x.chooseOption("Chat details");
  assert.equal(x.events.at(-1)?.[1], "AssistantDetails");
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  x.options();
  x.chooseOption("Voice preferences");
  assert.equal(x.voice()!.props.preferencesOpen, true);
  x.voice()!.props.onPreferencesClose();
  x.harness.flush();
  assert.equal(x.voice()!.props.preferencesOpen, false);
  assert.equal(x.alerts.length, 0);
  x.harness.unmount();
});

test("the chat menu routes each option after dismissal and follows notification availability", () => {
  const x = setup(A);
  x.options();
  assert.deepEqual(
    x.menuItems().map((n) => n.props.accessibilityLabel),
    ["Chat details", "Settings"],
  );
  x.chooseOption("Settings");
  assert.deepEqual(x.events.at(-1), [
    "dispatch",
    workspaceNavigateAction("AccountSettings"),
  ]);
  x.setBell(true);
  x.options();
  assert.deepEqual(
    x.menuItems().map((n) => n.props.accessibilityLabel),
    ["Chat details", "Notifications", "Settings"],
  );
  const select = x
    .menuItems()
    .find((n) => n.props.accessibilityLabel === "Notifications")!.props.onPress;
  select();
  assert.equal(
    x.events.some(
      ([type, route]) => type === "push" && route === "Notifications",
    ),
    false,
  );
  x.finishMenuDismissal();
  x.finishMenuDismissal();
  assert.equal(
    x.events.filter(
      ([type, route]) => type === "push" && route === "Notifications",
    ).length,
    1,
  );
  assert.equal(x.alerts.length, 0);
  x.harness.unmount();
});

test("an account change while More is open makes its Preferences and Details entries inert", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  x.options();
  const menu = x.menuItems();
  const before = x.events.length;
  x.setEpoch(8);
  for (const action of menu) {
    action.props.onPress();
    x.finishMenuDismissal();
  }
  x.harness.flush();
  assert.equal(x.events.length, before);
  x.harness.unmount();
});

test("Android Back restores the capsule, and tapping an option twice navigates once", () => {
  const x = setup(A, false, "android");
  x.options();
  elements(x.headerRight())
    .find((n) => n.type === "Modal")!
    .props.onRequestClose();
  assert.deepEqual(
    x.headerActions().map((n) => n.props.accessibilityLabel),
    ["New chat", "Chat options"],
  );
  x.options();
  const select = x
    .menuItems()
    .find((n) => n.props.accessibilityLabel === "Chat details")!.props.onPress;
  select();
  select();
  x.finishMenuDismissal();
  assert.equal(
    x.events.filter(
      ([type, route]) => type === "push" && route === "AssistantDetails",
    ).length,
    1,
  );
  x.harness.unmount();
});

test("closed voice surface callbacks and native menu actions cannot affect a newly opened call", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  const stale = x.voice()!.props;
  x.options();
  const oldMenu = x.menuItems();
  stale.onClose();
  x.harness.flush();
  x.headerRight();
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  x.voice()!.props.onSnapshot(liveVoice("current call"));
  stale.onSnapshot(liveVoice("old call"));
  stale.onClose();
  for (const item of oldMenu) {
    item.props.onPress();
    x.finishMenuDismissal();
  }
  x.harness.flush();
  assert.ok(x.voice());
  assert.equal(x.voice()!.props.preferencesOpen, false);
  assert.equal(
    x.all().find((node) => node.type === "FlatList")!.props.data.length,
    1,
  );
  assert.equal(
    x.all().find((node) => node.type === "FlatList")!.props.data[0].text,
    "current call",
    "a closed surface cannot replace the reopened call's live transcript",
  );
  assert.equal(
    x.events.some((event) => event[0] === "push"),
    false,
  );
  const footer = elements(
    expand(
      x.all().find((node) => node.type === "FlatList")!.props
        .ListFooterComponent,
    ),
  );
  assert.equal(
    footer.some(
      (node) =>
        node.props.accessibilityLabel === "Your speech is being transcribed",
    ),
    false,
  );
  x.harness.unmount();
});

test("reading voice history retains native row anchoring when older messages are prepended", async () => {
  const x = setup(A);
  const row = (id: string, seq: number) =>
    messageSchema.parse({
      id,
      seq,
      turn_id: "turn",
      role: "user",
      text: id,
      status: "completed",
      error_code: null,
      created_at: "2026-11-09T01:00:00Z",
    });
  const visible = row("visible", 20);
  x.assistant.history!.messages = [visible];
  x.harness.render();
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  x.pauseTranscript();
  const list = () => x.all().find((node) => node.type === "FlatList")!;
  assert.equal(
    list().props.maintainVisibleContentPosition?.minIndexForVisible,
    0,
  );
  x.assistant.history!.messages = [row("older", 19), visible];
  x.harness.render();
  assert.equal(
    list().props.maintainVisibleContentPosition?.minIndexForVisible,
    0,
  );
  assert.equal(list().props.data[1], visible);
  assert.ok(x.voice());
  x.harness.unmount();
});

test("eligible native composers expose distinct attachments, dictation and voice actions", () => {
  for (const id of [undefined, A]) {
    const x = setup(id, voiceSelection.NATIVE_VOICE_ENTRY_ENABLED);
    assert.equal(x.button("Open voice call")?.props.kind, "voice");
    assert.equal(x.voice(), undefined);
    assert.ok(x.button("Attach photos or files, or take a photo"));
    assert.ok(x.button("Dictate message"));
    assert.equal(x.featureQueries.at(-1)?.enabled, true);
    x.button("Attach photos or files, or take a photo")!.props.onPress();
    assert.equal(x.events.at(-1)?.[0], "choose-attachment");
    assert.equal(x.drafts.length, 0);
    x.harness.unmount();
  }
});

test("the chat screen routes authoritative confirmations through compact history while retaining pending actions", () => {
  const x = setup(A, false);
  const row = acknowledgementSchema.parse({
    id: "d9428888-122b-41e1-85c5-61cd3cbb3210",
    kind: "action",
    status: "pending",
    summary: "Create Twitter assistant",
    service_name: null,
    service_slug: null,
    tool_name: "nyxid__spawn_subagent",
    created_at: "2026-10-16T00:00:00Z",
    expires_at: "2099-10-16T00:15:00Z",
  });
  x.assistant.history!.acknowledgements = [row];
  x.harness.render();
  const footer = () =>
    elements(
      expand(
        x.all().find((node) => node.type === "FlatList")!.props
          .ListFooterComponent,
      ),
    );
  let confirmations = footer().find(
    (node) => node.type === "AssistantAcknowledgements",
  )!;
  assert.equal(confirmations.key, A);
  assert.equal(confirmations.props.acknowledgements[0], row);
  const pending = elements(expand(confirmations.props.renderPending(row)));
  assert.equal(
    pending.find((node) => node.type === "AssistantAcknowledgementCard")?.props
      .row,
    row,
  );
  assert.equal(
    typeof pending.find((node) => node.type === "AssistantAcknowledgementCard")
      ?.props.onDecision,
    "function",
  );
  row.status = "used";
  x.harness.render();
  confirmations = footer().find(
    (node) => node.type === "AssistantAcknowledgements",
  )!;
  assert.equal(confirmations.props.acknowledgements[0].status, "used");
  assert.equal(confirmations.props.renderPending(row), null);
  assert.equal(
    footer().some((node) => node.type === "Card"),
    false,
  );
  x.harness.unmount();
});

test("empty chat creates exactly one real draft and adopts it before opening voice", async () => {
  const x = setup();
  const open = x.button("Open voice call")!;
  open.props.onPress();
  open.props.onPress();
  x.harness.flush();
  assert.equal(x.drafts.length, 1);
  assert.equal(x.drafts[0]!.agentId, "agent-a");
  assert.equal(x.drafts[0]!.signal.aborted, false);
  assert.equal(x.button("Open voice call")?.props.disabled, true);
  assert.equal(x.voice(), undefined);
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, A);
  assert.deepEqual(
    x.events.filter(([name]) => name === "open"),
    [["open", A, undefined]],
  );
  assert.deepEqual(
    x.events.find(([name]) => name === "params"),
    [
      "params",
      {
        conversationId: A,
        acknowledgementId: undefined,
        home: false,
        draft: false,
        draftId: undefined,
        openLatest: false,
      },
    ],
  );
  assert.equal(
    x.events.some(([name]) => name === "send"),
    false,
  );
  x.harness.unmount();
});

test("a hung voice draft releases its spinner with a localized error and a retry ignores its late result", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const timeoutMessage = "无法打开语音，请重试。";
  const x = setup(undefined, true, "android", false, {
    "Voice could not open. Try again.": timeoutMessage,
  });
  x.button("Open voice call")!.props.onPress();
  x.harness.flush();
  context.mock.timers.tick(VOICE_RECOVERY_TIMEOUT_MS - 1);
  await x.harness.settle();
  assert.equal(x.button("Open voice call")?.props.disabled, true);
  assert.equal(x.drafts[0]!.signal.aborted, false);
  context.mock.timers.tick(1);
  await x.harness.settle();
  assert.equal(x.drafts[0]!.signal.aborted, true);
  assert.equal(x.voice(), undefined);
  assert.ok(x.input());
  assert.equal(x.button("Open voice call")?.props.disabled, false);
  assert.ok(x.all().some((node) => node.props.message === timeoutMessage));
  assert.equal(
    x.events.some(([name]) => name === "open" || name === "params"),
    false,
  );

  x.button("Open voice call")!.props.onPress();
  x.harness.flush();
  assert.equal(x.drafts.length, 2);
  assert.equal(x.button("Open voice call")?.props.disabled, true);
  assert.equal(
    x.all().some((node) => node.props.message === timeoutMessage),
    false,
  );
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.voice(), undefined, "timed-out success cannot adopt a chat");
  assert.equal(
    x.events.some(([name]) => name === "open" || name === "params"),
    false,
  );
  x.drafts[1]!.response.resolve(B);
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, B);
  assert.deepEqual(
    x.events.filter(([name]) => name === "open"),
    [["open", B, undefined]],
  );
  context.mock.timers.tick(VOICE_RECOVERY_TIMEOUT_MS);
  await x.harness.settle();
  assert.equal(x.drafts[1]!.signal.aborted, false);
  assert.equal(x.voice()?.props.thread, B);
  x.harness.unmount();
});

test("chat, account, blur and unmount cancel a hung draft without waiting for transport and release the entry", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  for (const reason of ["chat", "account", "blur", "unmount"] as const) {
    const x = setup();
    x.button("Open voice call")!.props.onPress();
    x.harness.flush();
    if (reason === "chat") {
      x.props.route.params.conversationId = B;
      x.assistant.openConversation(B);
      x.harness.render();
    } else if (reason === "account") {
      x.setEpoch(8);
      x.harness.render();
    } else if (reason === "blur") x.setFocused(false);
    else x.harness.unmount();
    await x.harness.settle();
    assert.equal(x.drafts[0]!.signal.aborted, true, reason);
    assert.equal(x.voice(), undefined, reason);

    if (reason !== "unmount") {
      if (reason === "blur") x.setFocused(true);
      assert.equal(x.button("Open voice call")?.props.disabled, false, reason);
      x.button("Open voice call")!.props.onPress();
      if (reason !== "chat") {
        assert.equal(x.drafts.length, 2, reason);
        x.drafts[1]!.response.resolve(B);
      }
      await x.harness.settle();
      assert.equal(x.voice()?.props.thread, B, reason);
    }
    x.draft.resolve(A);
    await x.harness.settle();
    assert.equal(
      x.events.some(([name, id]) => name === "open" && id === A),
      false,
      reason,
    );
    context.mock.timers.tick(VOICE_RECOVERY_TIMEOUT_MS);
    await x.harness.settle();
    if (reason !== "unmount") {
      assert.equal(x.voice()?.props.thread, B, reason);
      x.harness.unmount();
    }
  }
});

test("saved chat opens voice directly without creating a new draft or replacing its identity", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.drafts.length, 0);
  assert.equal(x.voice()?.props.thread, A);
  assert.equal(
    x.events.some(([name]) => name === "open"),
    false,
  );
  assert.equal(
    x.events.some(([name]) => name === "params"),
    false,
  );
  x.harness.unmount();
});

test("voice waits for the visible keyboard to hide while keeping the text input mounted", async () => {
  const x = setup(A, true, "android");
  x.keyboard.show();
  const input = x.input();
  const open = x.button("Open voice call")!;
  open.props.onPress();
  open.props.onPress();
  x.harness.flush();
  assert.equal(x.keyboard.listeners(), 1);
  assert.equal(x.input().props.value, input.props.value);
  assert.equal(x.voice(), undefined);
  assert.equal(x.button("Open voice call")?.props.disabled, true);
  assert.deepEqual(
    x.events.filter(([name]) => name === "dismissKeyboard"),
    [["dismissKeyboard"]],
  );
  x.keyboard.hide();
  await x.harness.settle();
  assert.equal(x.keyboard.listeners(), 0);
  assert.equal(x.voice()?.props.thread, A);
  assert.equal(x.input(), undefined);
  x.harness.unmount();
});

test("keyboard hide is subscribed before dismiss even when native dismissal completes synchronously", async () => {
  const x = setup(A, true, "android");
  x.keyboard.show();
  x.keyboard.hideOnDismiss();
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, A);
  assert.equal(x.keyboard.listeners(), 0);
  x.harness.unmount();
});

test("new chat creates one draft only after keyboard hide acknowledgement", async () => {
  const x = setup(undefined, true, "android");
  x.keyboard.show();
  const open = x.button("Open voice call")!;
  open.props.onPress();
  open.props.onPress();
  x.harness.flush();
  assert.equal(x.drafts.length, 0);
  assert.ok(x.input());
  x.keyboard.hide();
  await x.harness.settle();
  assert.equal(x.drafts.length, 1);
  assert.equal(x.voice(), undefined);
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, A);
  x.harness.unmount();
});

test("keyboard handoff times out with a usable composer and a retry ignores the late hide", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const timeoutMessage = "无法打开语音，请重试。";
  const x = setup(A, true, "android", false, {
    "Voice could not open. Try again.": timeoutMessage,
  });
  x.keyboard.show();
  x.button("Open voice call")!.props.onPress();
  context.mock.timers.tick(1499);
  await x.harness.settle();
  assert.equal(x.voice(), undefined);
  assert.equal(x.button("Open voice call")?.props.disabled, true);
  context.mock.timers.tick(1);
  await x.harness.settle();
  assert.equal(x.voice(), undefined);
  assert.ok(x.input());
  assert.equal(x.button("Open voice call")?.props.disabled, false);
  assert.equal(x.keyboard.listeners(), 0);
  assert.ok(x.all().some((node) => node.props.message === timeoutMessage));
  x.keyboard.hide();
  await x.harness.settle();
  assert.equal(x.voice(), undefined, "late acknowledgement cannot open voice");
  x.keyboard.show();
  x.button("Open voice call")!.props.onPress();
  x.harness.flush();
  assert.equal(
    x.all().some((node) => node.props.message === timeoutMessage),
    false,
  );
  x.keyboard.hide();
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, A);
  assert.equal(x.keyboard.listeners(), 0);
  x.harness.unmount();
});

test("chat, account, blur and unmount cancel the keyboard handoff without creating a draft", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  for (const reason of ["chat", "account", "blur", "unmount"] as const) {
    const x = setup(undefined, true, "android");
    x.keyboard.show();
    x.button("Open voice call")!.props.onPress();
    x.harness.flush();
    assert.equal(x.keyboard.listeners(), 1, reason);
    if (reason === "chat") {
      x.props.route.params.conversationId = B;
      x.assistant.conversationId = B;
      x.harness.render();
    } else if (reason === "account") {
      x.setEpoch(8);
      x.harness.render();
    } else if (reason === "blur") x.setFocused(false);
    else x.harness.unmount();
    assert.equal(x.keyboard.listeners(), 0, reason);
    x.keyboard.hide();
    context.mock.timers.tick(2000);
    if (reason === "unmount")
      await new Promise<void>((resolve) => setImmediate(resolve));
    else await x.harness.settle();
    assert.equal(x.drafts.length, 0, reason);
    assert.equal(x.voice(), undefined, reason);
    assert.equal(
      x.events.some(([name]) => name === "open" || name === "params"),
      false,
      reason,
    );
    if (reason !== "unmount") {
      assert.equal(
        x
          .all()
          .some(
            (node) => node.props.message === "Voice could not open. Try again.",
          ),
        false,
        reason,
      );
      if (reason === "blur") x.setFocused(true);
      assert.equal(x.button("Open voice call")?.props.disabled, false, reason);
      x.harness.unmount();
    }
  }
});

test("typing or losing voice availability during keyboard handoff cannot start a draft", async () => {
  for (const reason of ["typing", "upload", "feature", "offline"] as const) {
    const x = setup(undefined, true, "android");
    x.keyboard.show();
    x.button("Open voice call")!.props.onPress();
    x.harness.flush();
    if (reason === "typing") x.input().props.onChangeText("Keep this text");
    else if (reason === "upload") x.uploads.picking = true;
    else if (reason === "feature") x.setFeatures([]);
    else x.assistant.online = false;
    x.harness.render();
    x.keyboard.hide();
    await x.harness.settle();
    assert.equal(x.drafts.length, 0, reason);
    assert.equal(x.voice(), undefined, reason);
    assert.equal(x.keyboard.listeners(), 0, reason);
    if (reason === "typing")
      assert.equal(x.input().props.value, "Keep this text");
    x.harness.unmount();
  }
});

test("voice entry without a visible keyboard keeps the existing zero-delay path on iOS and Android", () => {
  for (const platform of ["ios", "android"]) {
    for (const id of [undefined, A]) {
      const x = setup(id, true, platform);
      x.button("Open voice call")!.props.onPress();
      x.harness.flush();
      assert.equal(x.keyboard.listeners(), 0, platform);
      if (id) assert.equal(x.voice()?.props.thread, id, platform);
      else assert.equal(x.drafts.length, 1, platform);
      x.harness.unmount();
    }
  }
});

test("voice slides up in the composer's place and closing it keeps the transcript", async () => {
  for (const initialId of [undefined, A]) {
    const x = setup(initialId);
    x.button("Open voice call")!.props.onPress();
    if (!initialId) x.draft.resolve(A);
    await x.harness.settle();
    assert.ok(
      x.all().some((child) => child.type === "FlatList"),
      "the chat stays on screen above the call",
    );
    assert.ok(x.voice(), "voice is mounted");
    assert.equal(
      x
        .all()
        .some(
          (element) =>
            element.props?.accessibilityLabel === "Message to assistant",
        ),
      false,
      "the voice composer takes the message composer's place",
    );
    const transcript = x.all().find((element) => element.type === "FlatList")!;
    const originalTranscriptKey = transcript.key;
    x.voice()!.props.onClose();
    x.harness.flush();
    assert.equal(x.voice(), undefined);
    assert.ok(
      x
        .all()
        .some(
          (element) =>
            element.props?.accessibilityLabel === "Message to assistant",
        ),
      "the message composer returns",
    );
    assert.equal(
      x.all().find((element) => element.type === "FlatList")!.key,
      originalTranscriptKey,
      "closing voice must preserve the existing transcript identity",
    );
    x.harness.unmount();
  }
});

test("voice entry yields to text, attachments, upload work and the active turn's Stop action", async () => {
  const x = setup(A);
  assert.equal(x.button("Open voice call")?.props.kind, "voice");
  x.input().props.onChangeText("Keep my draft");
  x.harness.flush();
  assert.equal(x.button("Open voice call"), undefined);
  assert.equal(x.button("Send message")?.props.kind, "send");
  assert.equal(x.button("Send message")?.props.disabled, false);
  x.input().props.onChangeText("");
  x.uploads.files = [
    { key: "file", handle: "file_1", kind: "file", file: { name: "note.txt" } },
  ];
  x.harness.render();
  assert.equal(x.button("Open voice call"), undefined);
  assert.equal(x.button("Send message")?.props.kind, "send");
  assert.equal(x.button("Send message")?.props.disabled, false);
  x.uploads.files = [];
  x.uploads.picking = true;
  x.harness.render();
  assert.equal(x.button("Open voice call"), undefined);
  assert.equal(x.button("Send message")?.props.disabled, true);
  x.uploads.picking = false;
  x.assistant.sending = true;
  x.harness.render();
  assert.equal(x.button("Open voice call"), undefined);
  assert.equal(x.button("Send message"), undefined);
  assert.equal(x.button("Stop assistant")?.props.kind, "stop");
  assert.equal(x.button("Stop assistant")?.props.disabled, undefined);
  x.button("Stop assistant")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.events.filter(([name]) => name === "stop").length, 1);
  assert.equal(x.drafts.length, 0);
  x.harness.unmount();
});

for (const kind of ["specialist", "nyxbot"] as const) {
  test(`an empty ${kind} composer keeps voice after its first text exchange`, async () => {
    const x = setup();
    x.assistant.agent.kind = kind;
    x.assistant.agent.name = kind === "specialist" ? "cake-shop" : "nyxbot";
    x.harness.render();
    assert.equal(x.button("Open voice call")?.props.disabled, false);
    x.input().props.onChangeText("Hello");
    x.harness.flush();
    assert.equal(x.button("Send message")?.props.disabled, false);
    x.button("Send message")!.props.onPress();
    await x.harness.settle();
    assert.equal(x.events.filter(([name]) => name === "send").length, 1);

    x.assistant.openConversation(A);
    x.adopt(A);
    Object.assign(x.assistant.history!.conversation, {
      role: kind === "specialist" ? "subagent" : "orchestrator",
      message_count: 2,
      active_turn: null,
    });
    x.harness.render();
    assert.equal(x.input().props.value, "");
    assert.equal(x.button("Open voice call")?.props.kind, "voice");
    assert.equal(x.button("Open voice call")?.props.disabled, false);
    assert.equal(x.button("Send message"), undefined);
    x.button("Open voice call")!.props.onPress();
    await x.harness.settle();
    assert.equal(x.voice()?.props.thread, A);
    assert.equal(x.drafts.length, 0, "saved chat voice must reuse its thread");
    x.harness.unmount();
  });
}

test("private attachments send directly without mention aliases or rewriting the draft", async () => {
  const x = setup(A);
  const file = {
    key: "report",
    handle: "file-1",
    kind: "file",
    file: { name: "report.txt" },
  };
  const text = "Please check @file-1 in this report";
  const payload: AssistantUploadedMessage = {
    conversationId: A,
    attachmentIds: ["11111111-1111-4111-8111-111111111111"],
    attachments: [],
  };
  x.uploads.files = [file];
  x.input().props.onChangeText(text);
  x.harness.flush();
  assert.equal(x.input().props.mentions, undefined);
  const pending = x
    .all()
    .find((element) => element.type === "AssistantPendingUploads")!;
  assert.equal(pending.props.showMentionHandles, false);
  pending.props.remove(file.key);
  x.harness.render();
  assert.equal(
    x.input().props.value,
    text,
    "removing a file must preserve the user's text",
  );
  assert.equal(x.uploads.files.length, 0);
  x.uploads.files = [file];
  x.uploads.session.submit = async (submit) => submit(payload);
  x.harness.render();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  assert.deepEqual(
    x.events.find(([name]) => name === "send"),
    ["send", text, payload],
  );
  assert.equal(x.input().props.value, "");
  x.harness.unmount();
});

test("the server feature flag and eligible chat context determine whether voice is shown", () => {
  const x = setup(A);
  x.setFeatures(undefined);
  assert.equal(x.button("Open voice call"), undefined);
  x.setFeatures([]);
  assert.equal(x.button("Open voice call"), undefined);
  x.setFeatures(["assistant:voice"]);
  assert.ok(x.button("Open voice call"));
  for (const context of [
    { channel: { platform: "telegram" } },
    { agent: { ...x.assistant.agent, destroyed: true } },
  ]) {
    Object.assign(x.assistant.history!.conversation, context);
    x.harness.render();
    assert.equal(x.button("Open voice call"), undefined);
    delete x.assistant.history!.conversation.channel;
    delete x.assistant.history!.conversation.role;
    x.assistant.history!.conversation.agent = x.assistant.agent;
  }
  x.harness.unmount();
});

test("switching chat while draft creation is pending aborts it and ignores its late result", async () => {
  const x = setup();
  x.button("Open voice call")!.props.onPress();
  x.props.route.params.conversationId = B;
  x.assistant.openConversation(B);
  x.harness.render();
  assert.equal(x.drafts[0]!.signal.aborted, true);
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.assistant.conversationId, B);
  assert.equal(x.voice(), undefined);
  assert.equal(
    x.events.some(([name]) => name === "params"),
    false,
  );
  assert.equal(
    x.events.some(([name, id]) => name === "open" && id === A),
    false,
  );
  x.harness.unmount();
});

test("switching account while draft creation is pending cannot adopt or open the old account's result", async () => {
  const x = setup();
  x.button("Open voice call")!.props.onPress();
  x.setEpoch(8);
  x.harness.render();
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.assistant.conversationId, undefined);
  assert.equal(x.voice(), undefined);
  assert.equal(
    x.events.some(([name]) => name === "open" || name === "params"),
    false,
  );
  x.harness.unmount();
});

test("typing during pending draft creation preserves the text and invalidates the voice launch", async () => {
  const x = setup();
  x.button("Open voice call")!.props.onPress();
  x.input().props.onChangeText("Actually, send this instead");
  x.harness.flush();
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.voice(), undefined);
  assert.equal(x.input().props.value, "Actually, send this instead");
  assert.equal(x.button("Send message")?.props.kind, "send");
  assert.equal(x.button("Send message")?.props.disabled, false);
  assert.equal(
    x.events.some(([name]) => name === "open" || name === "params"),
    false,
  );
  x.harness.unmount();
});

test("a pending voice draft cannot take over a composer that started uploading", async () => {
  const x = setup();
  x.button("Open voice call")!.props.onPress();
  x.uploads.picking = true;
  x.harness.render();
  assert.equal(x.button("Open voice call"), undefined);
  x.draft.resolve(A);
  await x.harness.settle();
  assert.equal(x.voice(), undefined);
  assert.equal(
    x.events.some(([name]) => name === "open" || name === "params"),
    false,
  );
  x.harness.unmount();
});

test("a late draft cannot open voice after the account loses voice availability", async () => {
  for (const reason of ["feature", "offline"] as const) {
    const x = setup();
    x.button("Open voice call")!.props.onPress();
    if (reason === "feature") x.setFeatures([]);
    else {
      x.assistant.online = false;
      x.harness.render();
    }
    x.draft.resolve(A);
    await x.harness.settle();
    assert.equal(x.voice(), undefined, reason);
    assert.equal(
      x.events.some(([name]) => name === "open" || name === "params"),
      false,
      reason,
    );
    x.harness.unmount();
  }
});

test("unmount aborts pending draft creation and late success cannot navigate", async () => {
  const x = setup();
  x.button("Open voice call")!.props.onPress();
  x.harness.unmount();
  assert.equal(x.drafts[0]!.signal.aborted, true);
  x.draft.resolve(A);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(
    x.events.some(([name]) => name === "open" || name === "params"),
    false,
  );
});

test("voice task review returns to its exact saved chat without closing voice", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  x.voice()!.props.onReview("ack-123");
  assert.deepEqual(
    x.events.find(([name]) => name === "open"),
    ["open", A, "ack-123"],
  );
  assert.deepEqual(
    x.events.find(([name]) => name === "params"),
    [
      "params",
      { conversationId: A, acknowledgementId: "ack-123", openLatest: false },
    ],
  );
  assert.ok(x.voice());
  x.harness.unmount();
});

test("reading the transcript keeps the compact voice composer and closing restores the entry", async () => {
  const x = setup(A);
  x.input().props.onChangeText(" ");
  x.harness.flush();
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.button("Open voice call"), undefined);
  x.voice()!.props.onUpdate();
  assert.equal(x.events.filter(([name]) => name === "recover").length, 1);
  const voiceKey = x.voice()!.key;
  const viewport = x.all().find((node) => "onTouchStart" in node.props)!;
  assert.equal(viewport.props.onTouchStart, undefined);
  x.all()
    .find((node) => node.type === "FlatList")!
    .props.onScrollBeginDrag();
  x.harness.flush();
  assert.equal(x.voice()!.key, voiceKey);
  for (const prop of [
    "minimized",
    "onExpand",
    "draft",
    "onChangeText",
    "onSendText",
    "canSendText",
  ])
    assert.equal(prop in x.voice()!.props, false);
  x.voice()!.props.onClose();
  x.harness.flush();
  assert.equal(x.voice(), undefined);
  assert.equal(x.input().props.value, " ");
  x.harness.unmount();
});

test("changing the selected chat removes its previous voice surface", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, A);
  x.props.route.params.conversationId = B;
  x.assistant.openConversation(B);
  x.harness.render();
  assert.equal(x.voice(), undefined);
  assert.ok(x.button("Open voice call"));
  x.harness.unmount();
});

test("edge measurements run after transcript follow handlers and preserve drag ownership", () => {
  const x = setup(A);
  const list = x.all().find((node) => node.type === "FlatList")!;
  list.props.onLayout({ nativeEvent: { layout: { height: 500 } } });
  list.props.onContentSizeChange(350, 900);
  list.props.onScroll({ nativeEvent: { contentOffset: { y: 100 } } });
  list.props.onScrollBeginDrag();
  assert.deepEqual(
    x.events.filter(
      ([name]) =>
        typeof name === "string" &&
        (name.startsWith("follow-") || name.startsWith("fade-")),
    ),
    [
      ["follow-layout", 500],
      ["fade-layout", 500],
      ["follow-content", 900],
      ["fade-content", 900],
      ["follow-scroll", 100],
      ["fade-scroll", 100],
      ["follow-drag"],
    ],
  );
  x.harness.unmount();
});

test("private user attribution joins only real via and steering context", () => {
  const x = setup(A, false);
  const render = x.all().find((node) => node.type === "FlatList")!.props
    .renderItem;
  const base = {
    id: "message",
    seq: 1,
    role: "user",
    text: "Continue",
    activities: [],
    attachments: [],
    created_at: "2026-10-27T00:00:00Z",
  };
  for (const [extra, expected] of [
    [{}, []],
    [
      { steering: { outcome: "applied" } },
      ["Steering · Guidance applied to this reply."],
    ],
    [
      { via: "Slack", steering: { outcome: "applied" } },
      ["Slack", "Steering · Guidance applied to this reply."],
    ],
    [{ via: "Slack" }, ["Slack"]],
  ] as const) {
    const row = expand(render({ item: { ...base, ...extra }, index: 0 }));
    const labels = elements(row)
      .filter((node) => node.type === "Label" && !node.props.mono)
      .map((node) => node.props.children.join(""));
    assert.deepEqual(labels, expected);
  }
  x.harness.unmount();
});

test("empty primary NyxBot uses home while specialists and contextual drafts keep their welcome", () => {
  const x = setup();
  const welcome = () =>
    elements(
      expand(
        x.all().find((node) => node.type === "FlatList")!.props
          .ListEmptyComponent,
      ),
    )
      .flatMap((node) => node.props.children)
      .filter((child: unknown) => typeof child === "string")
      .join(" ");
  assert.ok(x.all().some((node) => node.type === "NyxBotHome"));
  assert.equal(
    x.all().some((node) => node.type === "FlatList"),
    false,
  );
  x.props.contextual = true;
  x.harness.render();
  assert.match(welcome(), /Hi, I'm NyxBot\./);
  assert.match(welcome(), /help you use NyxID, manage your NyxID services/);
  x.assistant.agent.kind = "specialist";
  x.harness.render();
  assert.doesNotMatch(welcome(), /NyxBot/);
  assert.match(welcome(), /How can I help\?/);
  x.assistant.loading = true;
  x.harness.render();
  const empty = x.all().find((node) => node.type === "FlatList")!.props
    .ListEmptyComponent;
  assert.equal(empty.type, "SkeletonRows");
  assert.equal(empty.props.accessibilityLabel, "Loading messages");
  x.harness.unmount();
});

test("chat machine receipts forward every desktop selector to the full-screen route", () => {
  const x = setup(A, false);
  const render = x.all().find((node) => node.type === "FlatList")!.props
    .renderItem;
  const receipt = {
    node_id: "node",
    machine_name: "Workstation",
    action: "computer.snapshot",
    status: "completed",
  };
  const row = expand(
    render({
      item: {
        id: "message",
        seq: 1,
        role: "assistant",
        text: "Done",
        status: "completed",
        activities: [
          {
            id: "tool",
            label: "machine",
            status: "completed",
            started_at: "now",
            machine: receipt,
          },
        ],
        attachments: [],
        created_at: "now",
      },
      index: 0,
    }),
  );
  const card = elements(row).find((node) => node.type === "MachineToolCard")!;
  assert.equal(card.props.conversationId, A);
  assert.deepEqual(card.props.receipt, receipt);
  const params = {
    nodeId: "node",
    conversationId: A,
    contextId: "context",
    display: "dev",
  };
  card.props.onOpenDesktop(params);
  assert.deepEqual(x.events.at(-1), ["navigate", "MachineDesktop", params]);
  x.harness.unmount();
});

test("home opens the exact request conversation and leaves Message NyxBot below its independent scroll view", () => {
  const x = setup();
  const home = x.all().find((node) => node.type === "NyxBotHome")!;
  assert.equal(x.input().props.placeholder, "Message NyxBot");
  home.props.scrollProps.onContentSizeChange(390, 1200);
  home.props.scrollProps.onScroll({
    nativeEvent: { contentOffset: { y: 250 } },
  });
  assert.deepEqual(x.events, [
    ["fade-content", 1200],
    ["fade-scroll", 250],
  ]);
  home.props.onChat("agent-a", A, "request-id");
  assert.deepEqual(x.events.slice(2), [
    ["open", A, "request-id"],
    [
      "params",
      {
        home: false,
        draftId: undefined,
        agentId: "agent-a",
        conversationId: A,
        acknowledgementId: "request-id",
        draft: false,
        openLatest: false,
      },
    ],
  ]);
  x.harness.render();
  assert.equal(
    x.all().some((node) => node.type === "NyxBotHome"),
    false,
  );
  assert.ok(x.all().some((node) => node.type === "FlatList"));
  x.harness.unmount();
});

test("home roster Chat reuses latest-thread navigation and crew navigation retains the primary draft", () => {
  const x = setup();
  const home = x.all().find((node) => node.type === "NyxBotHome")!;
  home.props.onOpenCrew("crew-id");
  assert.deepEqual(JSON.parse(JSON.stringify(x.events)), [
    ["navigate", "GroupChat", { groupId: "crew-id" }],
  ]);
  home.props.onChat("agent-a");
  assert.deepEqual(x.events.at(-1), [
    "params",
    {
      home: false,
      draftId: x.props.route.params.draftId,
      agentId: "agent-a",
      conversationId: undefined,
      acknowledgementId: undefined,
      draft: false,
      openLatest: true,
    },
  ]);
  x.harness.unmount();
});

test("the folded Duo's agent picker is the workspace bottom sheet ending before the rail; elsewhere a native page sheet", () => {
  const x = setup(A);
  x.setWidth(678);
  x.setCornerSensor(104);
  x.chooseAgent();
  const modal = () =>
    x
      .all()
      .find((n) => n.type === "Modal" && n.props.visible && n.props.children)!;
  // A transparent Modal hosting the shared surface (its own travel, scrim,
  // handle, and the rail reserve), never a full-window page sheet.
  assert.equal(modal().props.transparent, true);
  assert.equal(modal().props.presentationStyle, undefined);
  assert.ok(x.all().some((n) => n.type === "WorkspaceSheetSurface"));
  assert.ok(x.all().some((n) => n.type === "WorkspaceAgentPicker"));
  x.harness.unmount();
  const phone = setup(A);
  phone.chooseAgent();
  assert.ok(
    phone
      .all()
      .some(
        (n) =>
          n.type === "Modal" &&
          n.props.visible &&
          n.props.presentationStyle === "pageSheet",
      ),
  );
  assert.equal(
    phone.all().some((n) => n.type === "WorkspaceSheetSurface"),
    false,
  );
  phone.harness.unmount();
});

test("Choose agent preserves its hierarchy and dismisses before pushing History or Details into main content", () => {
  for (const [label, route] of [
    ["History", "AssistantHistory"],
    ["Details", "AssistantDetails"],
  ]) {
    const x = setup(A);
    x.chooseAgent();
    const hierarchy = x.all().find((n) => n.type === "WorkspaceAgentPicker")!;
    assert.equal(hierarchy.props.visible, true);
    assert.equal(hierarchy.props.current.agentId, "agent-a");
    assert.equal(hierarchy.props.current.conversationId, A);
    const labels = x
      .all()
      .filter((n) => n.type === "Label")
      .flatMap((n) => n.props.children.flat(Infinity));
    assert.ok(labels.includes("This thread"));
    assert.ok(labels.includes("Saved chat"));
    assert.ok(
      labels.some(
        (label) => typeof label === "string" && label.includes("12 messages"),
      ),
    );
    const button = x
      .all()
      .find((n) => n.type === "ActionButton" && n.props.label === label)!;
    button.props.onPress();
    button.props.onPress();
    x.harness.flush();
    assert.equal(
      x.all().some((n) => n.type === "WorkspaceAgentPicker"),
      false,
    );
    assert.equal(
      x.events.some((e) => e[0] === "push"),
      false,
    );
    const modal = x.all().find((n) => n.type === "Modal")!;
    assert.equal(modal.props.visible, false);
    modal.props.onDismiss();
    modal.props.onDismiss();
    const pushed = x.events.filter((e) => e[0] === "push");
    assert.equal(pushed.length, 1);
    assert.equal(pushed[0]![1], route);
    assert.equal((pushed[0]![2] as any).agentId, "agent-a");
    if (label === "Details")
      assert.equal((pushed[0]![2] as any).conversationId, A);
    x.harness.unmount();
  }
});

test("header Details uses the latest conversation even when header identity stays unchanged", () => {
  const x = setup(A);
  x.adopt(B);
  x.options();
  x.chooseOption("Chat details");
  const pushed = x.events.find((e) => e[0] === "push")!;
  assert.equal(pushed[1], "AssistantDetails");
  assert.equal((pushed[2] as any).conversationId, B);
  x.harness.unmount();
});

test("an account change while the picker dismisses cancels its queued content destination", () => {
  const x = setup(A);
  x.chooseAgent();
  x.all()
    .find((n) => n.type === "ActionButton" && n.props.label === "History")!
    .props.onPress();
  x.harness.flush();
  x.setEpoch(8);
  x.all()
    .find((n) => n.type === "Modal")!
    .props.onDismiss();
  assert.equal(
    x.events.some((e) => e[0] === "push"),
    false,
  );
  x.harness.unmount();
});

test("picker retains each saved thread draft and starts an independent empty compose session", () => {
  const x = setup(A);
  x.input().props.onChangeText("Unsent review");
  x.harness.flush();
  x.chooseAgent();
  x.all()
    .find((node) => node.type === "WorkspaceAgentPicker")!
    .props.onSelect({ kind: "agent", agentId: "agent-a", conversationId: A });
  x.harness.flush();
  assert.equal(x.input().props.value, "Unsent review");
  assert.equal(
    x.events.some((event) => (event as unknown[])[0] === "open"),
    false,
  );
  x.chooseAgent();
  x.all()
    .find((node) => node.type === "WorkspaceAgentPicker")!
    .props.onSelect({ kind: "agent", agentId: "agent-a", conversationId: B });
  x.harness.flush();
  assert.equal(x.input().props.value, "");
  assert.equal(x.assistant.conversationId, B);
  assert.deepEqual(x.events.at(-1), [
    "params",
    {
      home: false,
      draftId: undefined,
      agentId: "agent-a",
      conversationId: B,
      acknowledgementId: undefined,
      draft: false,
      openLatest: false,
    },
  ]);
  x.chooseAgent();
  x.all()
    .find((node) => node.type === "WorkspaceAgentPicker")!
    .props.onSelect({ kind: "agent", agentId: "agent-a" });
  x.harness.flush();
  assert.deepEqual(x.events.at(-1), [
    "params",
    {
      home: false,
      draftId: x.props.route.params.draftId,
      agentId: "agent-a",
      conversationId: undefined,
      acknowledgementId: undefined,
      draft: true,
      openLatest: false,
    },
  ]);
  x.harness.unmount();
});

test("the real transcript inserts a context-reset note at its boundary and renders continuations as system lines", () => {
  const x = setup(A, false);
  const messages = [
    {
      id: "before",
      seq: 1,
      turn_id: "t1",
      role: "assistant",
      text: "Earlier",
      status: "completed",
      error_code: null,
      created_at: "2026-10-27T00:00:00Z",
      activities: [],
      attachments: [],
    },
    {
      id: "after",
      seq: 2,
      turn_id: "t2",
      role: "assistant",
      text: "Later",
      status: "completed",
      error_code: null,
      created_at: "2026-10-28T00:00:00Z",
      activities: [],
      attachments: [],
    },
  ];
  Object.assign(x.assistant.history!, { messages });
  Object.assign(x.assistant.history!.conversation, {
    context_reset_at: "2026-10-27T12:00:00Z",
    context_reset_reason: "model_changed",
  });
  x.harness.render();
  const render = x.all().find((node) => node.type === "FlatList")!.props
    .renderItem;
  const earlier = elements(expand(render({ item: messages[0], index: 0 })));
  assert.ok(
    !earlier.some(
      (node) => node.props.accessibilityLabel === "Conversation context reset",
    ),
  );
  const later = elements(expand(render({ item: messages[1], index: 1 })));
  assert.ok(
    later.some(
      (node) => node.props.accessibilityLabel === "Conversation context reset",
    ),
  );
  const continuation = expand(
    render({ item: { ...messages[1], continuation_of: "ack" }, index: 1 }),
  );
  assert.equal(
    continuation.props.children.join(""),
    "Continuing after your decision",
  );
  x.harness.unmount();
});

test("settled decisions fold into their exact recorded turn and CMA operations follow that turn", () => {
  const x = setup(A, false);
  const message = {
    id: "reply",
    seq: 2,
    turn_id: "turn-2",
    role: "assistant",
    text: "Result",
    status: "completed",
    error_code: null,
    created_at: "2026-10-28T00:00:00Z",
    activities: [
      {
        id: "tool-2",
        label: "read",
        status: "completed",
        started_at: "2026-10-28T00:00:00Z",
      },
    ],
    attachments: [],
  };
  const receipt = acknowledgementSchema.parse({
    id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    kind: "machine",
    status: "used",
    summary: "Read the file",
    creation_turn_id: "turn-2",
    activity_id: "tool-2",
    decided_by: "access_mode",
    service_slug: null,
    service_name: null,
    tool_name: "read",
    created_at: message.created_at,
    expires_at: "2099-01-01T00:00:00Z",
  });
  Object.assign(x.assistant.history!, {
    messages: [message],
    acknowledgements: [receipt],
    cma_operations: [{ id: "cma", turn_id: "turn-2" }],
  });
  x.harness.render();
  const list = x.all().find((node) => node.type === "FlatList")!;
  const row = elements(
    expand(list.props.renderItem({ item: message, index: 0 })),
  );
  assert.equal(
    row.find((node) => node.type === "AssistantActivityDisclosure")?.props
      .decisions[0],
    receipt,
  );
  assert.equal(
    row.find((node) => node.type === "AssistantCmaOperation")?.props.operation
      .id,
    "cma",
  );
  const footer = elements(expand(list.props.ListFooterComponent));
  assert.equal(
    footer.find((node) => node.type === "AssistantAcknowledgements")?.props
      .acknowledgements.length,
    0,
  );
  x.harness.unmount();
});

test("private, specialist and contextual header identities open the picker without changing the draft", () => {
  for (const mode of ["nyxbot", "specialist", "contextual"] as const) {
    const x = setup(A, false);
    // Wide bars float the same compact glass pill as narrow bars (covered
    // below); the thread line stays in the accessibility label.
    x.setWidth(1024);
    if (mode !== "nyxbot") {
      x.assistant.agent.kind = "specialist";
      x.assistant.agent.name = "researcher";
      x.assistant.agent.display_name = "Research team agent";
      x.props.contextual = mode === "contextual";
    }
    x.input().props.onChangeText("Unsent draft");
    x.harness.flush();
    const identity = elements(x.header()).find(
      (n) => n.type === "FloatingButton",
    )!;
    assert.match(
      identity.props.accessibilityLabel,
      /Chatting with .*Saved chat.*Choose agent/,
    );
    assert.ok(
      elements(x.header()).some(
        (n) => n.type === "AgentAvatar" && n.props.size === "lg",
      ),
    );
    identity.props.onPress();
    x.harness.flush();
    assert.equal(x.input().props.value, "Unsent draft");
    assert.ok(
      x
        .all()
        .some((n) =>
          [n.props.children].flat(Infinity).includes("Choose agent"),
        ),
    );
    assert.equal(x.events.filter((e) => e[0] === "open").length, 0);
    x.harness.unmount();
  }
  const draft = setup(undefined, false);
  draft.props.route.params = { home: false, draft: true };
  draft.harness.render();
  assert.match(
    elements(draft.header()).find((n) => n.type === "FloatingButton")!.props
      .accessibilityLabel,
    /NyxBot, New chat\. Choose agent/,
  );
  draft.harness.unmount();
});

test("Chat replaces the shared two-button capsule with a menu at its top-right corner", () => {
  for (const width of [390, 1024]) {
    const x = setup(undefined, false);
    x.setWidth(width);
    x.setBell(true);
    const labels = () =>
      x.headerActions().map((n) => n.props.accessibilityLabel);
    const capsule = elements(x.headerRight()).find(
      (n) => n.type === "FloatingCapsule",
    )!;
    assert.deepEqual(
      elements(capsule)
        .filter((n) => n.type === "FloatingItem")
        .map((n) => n.props.accessibilityLabel),
      ["New chat", "Chat options"],
    );
    for (const dismiss of ["outside", "back", "escape"]) {
      x.options();
      assert.deepEqual(labels(), []);
      const nodes = elements(x.headerRight());
      const menu = nodes.find((n) => n.props.accessibilityRole === "menu")!;
      const style = Object.assign({}, ...[menu.props.style].flat(Infinity));
      assert.equal(style.top, 66);
      assert.equal(style.right, 12);
      if (dismiss === "outside")
        nodes
          .find((n) => n.props.accessibilityLabel === "Close menu")!
          .props.onPress();
      if (dismiss === "back")
        nodes.find((n) => n.type === "Modal")!.props.onRequestClose();
      if (dismiss === "escape") menu.props.onAccessibilityEscape();
      x.finishMenuDismissal();
      assert.deepEqual(labels(), ["New chat", "Chat options"]);
      assert.equal(x.menuItems().length, 0);
    }
    assert.equal(
      x.events.some(([kind]) => kind === "push" || kind === "dispatch"),
      false,
    );
    x.harness.unmount();
  }
});

test("on the Duo this chat's controls go to the column in both modes: Menu, the agent's picker and More, with the bar's handlers", () => {
  const x = setup(A, false);
  const column = x.columnHeader();
  // Menu on the primary chat (contextual chats get Back), like the bar.
  assert.equal(column.role, "menu");
  assert.equal(column.disabled, false);
  // The agent: its avatar and one complete translated phrase (this test's
  // translator returns the template; the name is its placeholder).
  assert.equal(column.identity.agent.id, "agent-a");
  assert.equal(column.identity.label, "{{name}}. Choose agent");
  assert.equal(column.more.label, "Chat options");
  // The agent opens the identity pill's picker (the same Modal).
  const modals = () =>
    x.all().filter((n) => n.type === "Modal" && n.props.visible).length;
  const before = modals();
  column.identity.onPress();
  x.harness.flush();
  assert.equal(modals(), before + 1);
  // More retains the base direct Details action outside a corner window.
  const alerts = x.alerts.length;
  column.more.onPress();
  x.harness.flush();
  assert.equal(x.alerts.length, alerts);
  assert.equal(
    x.events.some((e) => e[0] === "push"),
    false,
    "Details waits for picker dismissal",
  );
  x.all()
    .find((n) => n.type === "Modal")!
    .props.onDismiss();
  x.harness.flush();
  assert.equal(x.events.at(-1)?.[1], "AssistantDetails");
  // A contextual chat (pushed from another page) has Back, like its bar.
  x.props.contextual = true;
  x.harness.render();
  assert.equal(x.columnHeader().role, "back");
  x.harness.unmount();
});

for (const platform of ["ios", "android"]) {
  for (const surface of ["home", "private", "contextual"]) {
    test(`${platform}: ${surface} enables the shared idle input and toolbar without corner hardware`, () => {
      const x = setup(surface === "private" ? A : undefined, false, platform);
      x.props.contextual = surface === "contextual";
      x.harness.render();
      assert.equal(x.input().props.compactWhenIdle, true);
      const layout = { compact: true, onLeadingWidth() {}, onTrailingWidth() {} };
      assert.equal(x.input().props.renderActions(undefined, layout).props.layout, layout);
      x.harness.unmount();
    });
  }
}

test("all idle composers collapse across phone, Duo and wide windows; attachment previews stay separate", () => {
  const x = setup(A, false);
  const idle = () => x.input().props.compactWhenIdle;
  assert.equal(idle(), true, "ordinary phone uses the shared idle row");
  const layout = { compact: true, onLeadingWidth() {}, onTrailingWidth() {} };
  assert.equal(x.input().props.renderActions(undefined, layout).props.layout, layout, "ordinary toolbar receives the same compact layout");
  x.setQuickRail(true);
  assert.equal(idle(), true, "folded Duo may collapse");
  x.uploads.files.push({ key: "photo", status: "uploaded" });
  x.harness.render();
  assert.equal(idle(), true, "attachments keep their own preview strip");
  x.uploads.files.length = 0;
  x.uploads.picking = true;
  x.harness.render();
  assert.equal(idle(), true, "the picker does not expand an idle text input");
  x.uploads.picking = false;
  x.setDictating(true);
  assert.equal(idle(), false, "dictation keeps its controls");
  x.setDictating(false);
  assert.equal(idle(), true);
  x.setWidth(1024);
  assert.equal(idle(), true, "wide windows use the same idle row");
  x.harness.unmount();
});

test("the Duo bar keeps a fixed avatar; its title is the one tail-truncating line before the actions", () => {
  const x = setup(A, false);
  x.setQuickRail(true);
  x.assistant.agent.display_name =
    "A very long specialist name that cannot fit beside three actions";
  x.harness.render();
  const identity = elements(x.header()).find(
    (n) => n.type === "FloatingButton",
  )!;
  // One shared glass pill around avatar, title and chevron; its glass hugs
  // the pill and yields width.
  const capsule = Object.assign(
    {},
    ...[identity.props.capsuleStyle].flat(Infinity).filter(Boolean),
  );
  assert.equal(capsule.alignSelf, "flex-start");
  assert.equal(capsule.maxWidth, "100%");
  const lines = elements(identity).filter((n) => n.type === "Text");
  const title = lines.find((n) => n.props.ellipsizeMode === "tail")!;
  assert.equal(
    lines.filter((n) => n.props.ellipsizeMode === "tail").length,
    1,
    "the thread line is omitted on narrow bars",
  );
  assert.equal(title.props.numberOfLines, 1);
  assert.equal(title.props.allowFontScaling, true);
  // Largest accessibility size: capped growth, still a single shrinking line.
  assert.ok(title.props.maxFontSizeMultiplier <= 1.35);
  const titleStyle = Object.assign({}, ...[title.props.style].flat());
  assert.equal(titleStyle.flexShrink, 1);
  assert.equal(
    titleStyle.lineHeight,
    undefined,
    "a fixed line box would clip scaled descenders",
  );
  const labels = elements(identity).find(
    (n) => n.type === "View" && n.props.style?.flexShrink === 1,
  );
  assert.ok(labels, "the label column shrinks instead of the actions");
  // ☰ | avatar + title ⌄ | actions: the avatar is a fixed 32pt mark.
  const order = elements(identity).map((n) =>
    n.type === "AgentAvatar"
      ? "avatar"
      : n === title
        ? "title"
        : n.type === "ChevronDown"
          ? "chevron"
          : undefined,
  );
  assert.deepEqual(order.filter(Boolean), ["avatar", "title", "chevron"]);
  const avatar = elements(identity).find((n) => n.type === "AgentAvatar")!;
  assert.equal(avatar.props.size, "lg");
  assert.equal(avatar.props.agent.id, "agent-a");
  x.harness.unmount();
});

test("on a corner-sensor phone the agent picker's header stays at the top; its full-width body starts below the cluster", () => {
  const x = setup(A, false);
  const sheet = () => {
    const modal = x.all().find((n) => n.type === "Modal")!;
    const panel = modal.props.children[0];
    return {
      panel,
      header: panel.props.children[0],
      body: panel.props.children[1],
    };
  };
  x.chooseAgent();
  const style = (node: { props: Record<string, any> }) =>
    Object.assign({}, ...[node.props.style].flat().filter(Boolean));
  const bodyStyle = () =>
    Object.assign(
      {},
      ...[sheet().body.props.contentContainerStyle ?? sheet().body.props.style]
        .flat()
        .filter(Boolean),
    );
  assert.equal(style(sheet().panel).paddingTop, 20);
  assert.equal(style(sheet().header).paddingHorizontal, 20);
  assert.equal(style(sheet().header).paddingRight, undefined);
  assert.equal(style(sheet().header).marginBottom, 0);
  x.setCornerSensor(83);
  // The header stays flush at its 20pt top padding and its trailing Close
  // stays left of the column; the full-width body starts at the measured
  // 152pt cluster bottom (20pt top + 44pt row + 88pt), rail or not.
  for (const rail of [false, true]) {
    x.setQuickRail(rail);
    assert.equal(style(sheet().panel).paddingTop, 20);
    assert.equal(style(sheet().header).paddingRight, 20 + 83);
    assert.equal(20 + 44 + style(sheet().header).marginBottom, 152);
    assert.equal(bodyStyle().paddingRight, undefined, "runs to the edge");
  }
  x.setQuickRail(false);
  x.setWidth(1024);
  assert.ok(
    x.all().some((n) => n.type === "WorkspaceSheetSurface"),
    "a wide Duo uses the same bottom-attached surface as Settings",
  );
  assert.equal(
    x.all().find((n) => n.type === "Modal")!.props.transparent,
    true,
    "the status column stays beside the sheet",
  );
  // An iPad still uses its native page sheet.
  x.setCornerSensor(0);
  assert.equal(x.all().some((n) => n.type === "WorkspaceSheetSurface"), false);
  assert.equal(style(sheet().panel).paddingTop, 20);
  assert.equal(style(sheet().header).marginBottom, 0);
  x.harness.unmount();
});

test("scrolling the transcript dismisses the keyboard and never swallows a first tap", () => {
  const x = setup(A, false);
  const list = x.all().find((n) => n.type === "FlatList")!;
  // On-drag dismissal; taps on links, message actions and cards act while
  // the keyboard is up instead of only dismissing it.
  assert.equal(list.props.keyboardDismissMode, "on-drag");
  assert.equal(list.props.keyboardShouldPersistTaps, "handled");
  x.harness.unmount();
});

test("P9 transcript keeps base native identity outside Duo and remeasures in corner windows", () => {
  const x = setup(A, false);
  const list = () => x.all().find((n) => n.type === "FlatList")!;
  const before = list().key;
  x.harness.render();
  assert.equal(
    list().key,
    before,
    "ordinary renders keep the list and its scroll",
  );
  x.setFontScale(3.12);
  assert.equal(list().key, before, "ordinary font changes preserve the list");
  x.setQuickRail(true);
  const duoBefore = list().key;
  x.setFontScale(2);
  assert.notEqual(list().key, duoBefore);
  assert.ok(String(list().key).startsWith(`${A}:`), "still scoped to its chat");
  x.harness.unmount();
});

test("header needs-you status takes precedence over active-turn running status", () => {
  const x = setup(A, false);
  const conversation = x.assistant.history!.conversation;
  conversation.active_turn = { id: "turn", tools: [] };
  x.harness.render();
  assert.match(
    elements(x.header()).find((n) => n.type === "FloatingButton")!.props
      .accessibilityLabel,
    /Running/,
  );
  x.assistant.history!.acknowledgements = [
    acknowledgementSchema.parse({
      id: "d9428888-122b-41e1-85c5-61cd3cbb3210",
      kind: "action",
      status: "pending",
      summary: "Needs a decision",
      service_slug: null,
      service_name: null,
      tool_name: null,
      created_at: "2026-10-28T00:00:00Z",
      expires_at: "2099-10-28T01:00:00Z",
    }),
  ];
  x.harness.render();
  assert.match(
    elements(x.header()).find((n) => n.type === "FloatingButton")!.props
      .accessibilityLabel,
    /Needs you/,
  );
  x.harness.unmount();
});

test("settled transcript keeps the compact footer without voting for completed/failed/stopped replies", () => {
  const x = setup(A, false);
  const item = {
    id: "reply",
    seq: 2,
    turn_id: "turn",
    role: "assistant",
    text: "Reply",
    status: "completed",
    error_code: null as string | null,
    created_at: "2026-10-27T01:00:00Z",
    activities: [],
    attachments: [],
  };
  const render = () => {
    const list = x.all().find((n) => n.type === "FlatList")!;
    return elements(expand(list.props.renderItem({ item, index: 0 }))).find(
      (n) => n.type === "AssistantMessageFooter",
    )!;
  };
  assert.equal(render().props.timestamp, item.created_at);
  assert.equal(render().props.onFeedback, undefined);
  item.status = "failed";
  assert.equal(render().props.onFeedback, undefined);
  item.status = "completed";
  item.error_code = "cancelled";
  assert.equal(render().props.onFeedback, undefined);
  x.harness.unmount();
});

test("home navigation keeps thread text in that thread and uses a separate NyxBot draft", () => {
  const x = setup(A, false);
  x.input().props.onChangeText("Original thread draft");
  x.harness.flush();
  x.props.route.params = { home: true, openLatest: false };
  x.assistant.openConversation();
  x.harness.render();
  assert.equal(x.input().props.value, "");
  x.input().props.onChangeText("Message NyxBot draft");
  x.harness.flush();
  x.props.route.params = { conversationId: A, agentId: x.assistant.agent.id };
  x.assistant.openConversation(A);
  x.harness.render();
  assert.equal(x.input().props.value, "Original thread draft");
  x.props.route.params = { home: true, openLatest: false };
  x.assistant.openConversation();
  x.harness.render();
  assert.equal(x.input().props.value, "Message NyxBot draft");
  x.harness.unmount();
});

test("header compose preserves the viewed agent and starts a normal empty chat from home or either agent's thread", () => {
  for (const origin of [
    "home",
    "nyxbot-thread",
    "specialist-thread",
    "nyxbot-new",
    "specialist-new",
  ]) {
    const saved = origin.endsWith("thread");
    const x = setup(saved ? A : undefined, false);
    const specialist = origin.startsWith("specialist");
    Object.assign(
      x.assistant.agent,
      specialist
        ? {
            id: "specialist",
            kind: "specialist",
            name: "researcher",
            display_name: "Researcher",
          }
        : {
            id: "agent-a",
            kind: "nyxbot",
            name: "nyxbot",
            display_name: "NyxBot",
          },
    );
    x.assistant.agentId = x.assistant.agent.id;
    x.props.route.params = {
      agentId: x.assistant.agent.id,
      conversationId: saved ? A : undefined,
      home: origin === "home",
      draft: !saved && origin !== "home",
    };
    x.harness.render();
    x.input().props.onChangeText("Keep the original draft");
    x.harness.flush();
    x.compose();
    assert.equal(x.props.route.params.agentId, x.assistant.agent.id, origin);
    assert.equal(x.props.route.params.home, false, origin);
    assert.equal(x.props.route.params.draft, true, origin);
    assert.ok(x.props.route.params.draftId, origin);
    assert.equal(x.input().props.value, "", origin);
    assert.equal(
      x.all().some((n) => n.type === "NyxBotHome"),
      false,
      origin,
    );
    assert.ok(
      x.all().some((n) => n.type === "FlatList"),
      origin,
    );
    assert.match(
      elements(x.header()).find((n) => n.type === "FloatingButton")!.props
        .accessibilityLabel,
      new RegExp(`Chatting with ${x.assistant.agent.display_name}, New chat`),
      origin,
    );
    const firstDraftId = x.props.route.params.draftId;
    const firstGeneration = x.input().props.generation;
    x.input().props.onChangeText("Keep second draft too");
    x.harness.flush();
    x.compose();
    assert.notEqual(x.props.route.params.draftId, firstDraftId);
    assert.notEqual(x.input().props.generation, firstGeneration);
    assert.equal(x.input().props.value, "");
    if (saved) {
      x.assistant.openConversation(A);
      x.props.route.params = {
        agentId: x.assistant.agent.id,
        conversationId: A,
      };
      x.harness.render();
      assert.equal(x.input().props.value, "Keep the original draft");
    }
    x.harness.unmount();
  }
});

test("agent picker New chat starts NyxBot and specialists in the normal empty-chat view", () => {
  for (const specialist of [false, true]) {
    const x = setup(A, false);
    if (specialist)
      Object.assign(x.assistant.agent, {
        id: "specialist",
        kind: "specialist",
        display_name: "Researcher",
      });
    x.assistant.agentId = x.assistant.agent.id;
    x.harness.render();
    x.input().props.onChangeText("Thread draft");
    x.harness.flush();
    x.chooseAgent();
    x.all()
      .find((n) => n.type === "WorkspaceAgentPicker")!
      .props.onSelect({ kind: "agent", agentId: x.assistant.agent.id });
    x.harness.flush();
    assert.equal(x.props.route.params.agentId, x.assistant.agent.id);
    assert.equal(x.props.route.params.home, false);
    assert.equal(x.input().props.value, "");
    assert.equal(
      x.all().some((n) => n.type === "NyxBotHome"),
      false,
    );
    x.harness.unmount();
  }
});

test("home composer sends to NyxBot and adoption leaves a normal chat with its own draft", async () => {
  const x = setup(undefined, false);
  assert.ok(x.all().some((n) => n.type === "NyxBotHome"));
  x.input().props.onChangeText("Message NyxBot");
  x.harness.flush();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  assert.deepEqual(
    x.events.find((e) => e[0] === "send"),
    ["send", "Message NyxBot", undefined],
  );
  x.adopt(A);
  assert.equal(x.props.route.params.home, false);
  assert.equal(x.props.route.params.conversationId, A);
  assert.equal(
    x.all().some((n) => n.type === "NyxBotHome"),
    false,
  );
  assert.equal(x.input().props.value, "");
  x.input().props.onChangeText("Draft for the adopted thread");
  x.harness.flush();
  x.props.route.params = { home: true };
  x.assistant.openConversation();
  x.harness.render();
  assert.equal(x.input().props.value, "");
  x.props.route.params = { home: false, conversationId: A };
  x.assistant.openConversation(A);
  x.harness.render();
  assert.equal(x.input().props.value, "Draft for the adopted thread");
  x.harness.unmount();
});

test("explicit NyxBot draft mode overrides the navigator's initial home parameter", () => {
  const x = setup(undefined, false);
  x.props.route.params = { home: true, draft: true };
  x.harness.render();
  assert.equal(
    x.all().some((n) => n.type === "NyxBotHome"),
    false,
  );
  assert.ok(x.all().some((n) => n.type === "FlatList"));
  x.harness.unmount();
});

test("History selection retains the source thread draft and restores the selected thread draft", () => {
  const x = setup(A, false);
  (x.assistant.conversations as any[]).push({
    id: B,
    title: "Other chat",
    agent: x.assistant.agent,
  });
  x.input().props.onChangeText("Original draft");
  x.harness.flush();
  x.chooseAgent();
  x.all()
    .find((n) => n.type === "ActionButton" && n.props.label === "History")!
    .props.onPress();
  x.harness.flush();
  assert.equal(x.input().props.value, "Original draft");
  x.all()
    .find((n) => n.type === "Modal")!
    .props.onDismiss();
  const pushed = x.events.find((e) => e[0] === "push")!;
  assert.equal(pushed[1], "AssistantHistory");
  // The shared history browser selects the thread through a root chat action.
  x.props.route.params = { home: false, conversationId: B, agentId: "agent-a" };
  x.assistant.openConversation(B);
  x.harness.render();
  assert.equal(x.input().props.value, "");
  assert.equal(x.props.route.params.home, false);
  x.input().props.onChangeText("Other draft");
  x.harness.flush();
  x.props.route.params = { conversationId: A };
  x.assistant.openConversation(A);
  x.harness.render();
  assert.equal(x.input().props.value, "Original draft");
  x.props.route.params = { conversationId: B };
  x.assistant.openConversation(B);
  x.harness.render();
  assert.equal(x.input().props.value, "Other draft");
  x.harness.unmount();
});

test("private/contextual transcript viewport spans the safe area while content keeps its inset column", () => {
  const x = setup("saved-chat");
  const page = x.all().find((n) => n.type === "WorkspacePage")!;
  const list = x.all().find((n) => n.type === "FlatList")!;
  const viewport = x.all().find((n) => n.props.children.includes(list))!;
  assert.equal(page.props.contentStyle.paddingHorizontal, 0);
  for (const style of [list.props.style, viewport.props.style]) {
    assert.equal(style.paddingHorizontal, undefined);
    assert.equal(style.marginHorizontal, undefined);
    assert.equal(style.maxWidth, undefined);
    assert.equal(style.flex, 1);
  }
  const content = Object.assign({}, ...list.props.contentContainerStyle);
  assert.equal(content.paddingHorizontal, 12);
  assert.equal(content.maxWidth, 782);
  assert.equal(content.width, "100%");
  assert.equal(content.alignSelf, "center");
  assert.equal(list.props.scrollIndicatorInsets, undefined);
  assert.ok(
    viewport.props.children.some((n: any) => n?.type === "JumpToLatest"),
  );
  assert.ok(
    viewport.props.children.some((n: any) => n?.type === "ScrollEdgeFades"),
  );
  x.harness.unmount();
});

test("private and contextual composers bind permissions only to the current agent/thread", () => {
  for (const contextual of [false, true]) {
    const x = setup(A);
    x.props.contextual = contextual;
    Object.assign(x.assistant.agent, {
      kind: "specialist",
      access_mode_override: "ask_every_time",
    });
    x.harness.render();
    const control = () => x.all().find((n) => n.type === "ComposerSettings")!;
    assert.equal(control().props.agentOverride, "ask_every_time");
    assert.equal(control().props.activeTurn, false);
    for (const field of ["model", "advanced", "onModelChanged"])
      assert.equal(control().props[field], undefined);
    x.assistant.history!.conversation.active_turn = { turn_id: "turn" };
    x.harness.render();
    assert.equal(control().props.activeTurn, true);
    x.harness.unmount();
  }
});

test("private connection selection uses the shared chooser, prefixes sends, blocks unavailable keys and suppresses voice", async () => {
  const x = setup(A);
  x.button("Attach photos or files, or take a photo")!.props.onPress();
  const extra = x.events.find(
    ([name]) => name === "choose-attachment",
  )![1] as any;
  assert.equal(extra[0].label, "Use a connection");
  extra[0].onPress();
  assert.ok(x.events.some(([name]) => name === "choose-connection"));
  x.connection.connection = { id: "key-a" };
  x.connection.prefix = (text) => `/service key-a\n${text}`;
  x.harness.render();
  assert.equal(x.button("Open voice call"), undefined);
  x.input().props.onChangeText("Use this account");
  x.harness.flush();
  x.connection.unavailable = true;
  x.harness.render();
  assert.equal(x.button("Send message")!.props.disabled, true);
  x.connection.unavailable = false;
  x.harness.render();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  assert.ok(
    x.events.some(
      ([name, text]) =>
        name === "send" && text === "/service key-a\nUse this account",
    ),
  );
  assert.ok(x.events.some(([name]) => name === "connection-sent"));
  x.harness.unmount();
});

test("capable running replies expose bounded text guidance beside Stop without using uploads or connection prefixes", async () => {
  const x = setup(A);
  x.assistant.history!.conversation.active_turn = {
    turn_id: "turn",
    steering_allowed: true,
  };
  x.guidance.canSteer = true;
  x.connection.prefix = () => {
    throw new Error("Guidance cannot use a connection prefix");
  };
  x.uploads.session.sending = true;
  x.harness.render();
  assert.equal(x.input().props.placeholder, "Guide this running reply…");
  assert.equal(x.input().props.editable, true);
  x.input().props.onChangeText("Only change the tests");
  x.harness.flush();
  assert.ok(x.button("Stop assistant"));
  const steer = x
    .all()
    .find((n) => n.props.accessibilityLabel === "Send guidance")!;
  assert.equal(steer.props.disabled, false);
  steer.props.onPress();
  await x.harness.settle();
  assert.ok(
    x.events.some(
      ([name, text, count]) =>
        name === "guidance" && text === "Only change the tests" && count === 0,
    ),
  );
  assert.equal(
    x.events.some(([name]) => name === "send" || name === "connection-sent"),
    false,
  );
  assert.equal(x.input().props.value, "");
  x.harness.unmount();
});

test("Home header names the dashboard without calling it a new chat", () => {
  const x = setup(undefined, false);
  x.props.route.params = { home: true, openLatest: false };
  x.harness.render();
  assert.match(
    elements(x.header()).find((node) => node.type === "FloatingButton")!.props
      .accessibilityLabel,
    /NyxBot, Home\. Choose agent/,
  );
  assert.ok(x.all().some((node) => node.type === "NyxBotHome"));
  x.compose();
  x.harness.flush();
  assert.match(
    elements(x.header()).find((node) => node.type === "FloatingButton")!.props
      .accessibilityLabel,
    /NyxBot, New chat\. Choose agent/,
  );
  assert.equal(
    x.all().some((node) => node.type === "NyxBotHome"),
    false,
  );
  x.harness.unmount();
});

test("Android picker destinations wait for the dismiss slide; reduced motion skips the delay", async () => {
  for (const reduced of [false, true]) {
    const x = setup(A, false, "android", reduced);
    x.chooseAgent();
    x.all()
      .find((n) => n.type === "ActionButton" && n.props.label === "History")!
      .props.onPress();
    x.harness.flush();
    assert.equal(
      x.events.some((e) => e[0] === "push"),
      false,
    );
    await new Promise((resolve) => setTimeout(resolve, reduced ? 10 : 380));
    assert.equal(x.events.filter((e) => e[0] === "push").length, 1);
    x.harness.unmount();
  }
});

test("Home displays a friendly 503 beside Reload agents and clears it after retry", async () => {
  const x = setup();
  x.assistant.agentsError = buildApiError(
    "<html><center>nginx</center></html>",
    503,
  ).message;
  x.harness.render();
  assert.ok(
    x
      .all()
      .some(
        (node) =>
          node.props.message ===
          "NyxID is temporarily unavailable. Try again in a moment.",
      ),
  );
  assert.doesNotMatch(JSON.stringify(x.harness.value()), /<html>|nginx/);
  const reload = x.all().find((node) => node.props.label === "Reload agents");
  assert.ok(reload);
  reload.props.onPress();
  await x.harness.settle();
  x.harness.render();
  assert.ok(x.events.some(([name]) => name === "reloadAgents"));
  assert.equal(
    x.all().some((node) => node.props.label === "Reload agents"),
    false,
  );
  assert.equal(
    x
      .all()
      .some(
        (node) =>
          node.props.message ===
          "NyxID is temporarily unavailable. Try again in a moment.",
      ),
    false,
  );
  x.harness.unmount();
});

test("2345 history failure keeps the composer usable, offers refresh, and respects destroyed state", async () => {
  const x = setup(A);
  x.assistant.error =
    "Conversation history could not be loaded. Please try again.";
  x.harness.render();
  assert.notEqual(x.input().props.editable, false);
  const refresh = x
    .all()
    .find(
      (node) =>
        node.props.label === "Refresh history" ||
        node.props.accessibilityLabel === "Refresh history",
    );
  assert.ok(refresh);
  await refresh.props.onPress();
  assert.ok(x.events.some(([name]) => name === "recover"));
  x.assistant.history!.conversation.agent = {
    ...x.assistant.agent,
    destroyed: true,
  };
  x.harness.render();
  assert.equal(x.input().props.editable, false);
  x.harness.unmount();
});

test("Home loading never registers a critical navigation guard; only its initial send or upload does", () => {
  const env = setup();
  env.assistant.loading = true;
  env.harness.render();
  assert.equal(env.navigationDisabled(), false);
  assert.equal(
    env
      .headerActions()
      .find((node) => node.props.accessibilityLabel === "New chat")?.props
      .disabled,
    true,
  );
  env.assistant.sending = true;
  env.harness.render();
  assert.equal(env.navigationDisabled(), true);
  env.assistant.sending = false;
  env.uploads.session.sending = true;
  env.harness.render();
  assert.equal(env.navigationDisabled(), true);
  env.adopt(A);
  assert.equal(
    env.navigationDisabled(),
    false,
    "an established chat can leave during a send",
  );
  env.harness.unmount();
});

test("P9 ordinary floating identity retains the base thread detail", () => {
  const x = setup(A, false);
  const lines = elements(x.header()).filter(
    (n) => n.type === "Text" && n.props.ellipsizeMode === "tail",
  );
  assert.equal(lines.length, 2, "name and thread details stay visible");
  x.harness.unmount();
});

test("private chat mounts the guarded attachment-source sheet returned by its upload session", () => {
  const x = setup("chat");
  const sourcePicker = { choices: [{ label: "Files", onPress() {} }], onClose() {} };
  x.uploads.sourcePicker = sourcePicker;
  x.harness.render();
  const picker = x.all().find(n => n.type === "AttachmentSourcePicker")!;
  assert.equal(picker.props.choices, sourcePicker.choices);
  assert.equal(picker.props.onClose, sourcePicker.onClose);
  x.harness.unmount();
});

test("the selected draft model reaches a first text-only send", async () => {
  const x = setup();
  x.uploads.session.model = "nyxagent/research";
  x.input().props.onChangeText("Compare these plans");
  x.harness.flush();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  assert.deepEqual(x.events.find(([name]) => name === "send"),
    ["send", "Compare these plans", undefined, "nyxagent/research"]);
  assert.equal(x.drafts.length, 0);
  x.harness.unmount();
});

test("a previously created attachment draft applies the selected model to that ID before sending", async () => {
  const x = setup();
  x.uploads.session.model = "nyxid/model-choice";
  const payload: AssistantUploadedMessage = {
    conversationId: A,
    attachmentIds: ["11111111-1111-4111-8111-111111111111"],
    attachments: [],
  };
  x.uploads.session.submit = async (submit) => submit(payload);
  x.input().props.onChangeText("Review this file");
  x.harness.flush();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  assert.deepEqual(x.events.filter(([name]) => name === "patch-draft-model" || name === "send"), [
    ["patch-draft-model", A, "nyxid/model-choice"],
    ["send", "Review this file", payload],
  ]);
  assert.equal(x.drafts.length, 0);
  x.harness.unmount();
});

test("a refused draft model PATCH prevents dispatch and preserves the message and model", async () => {
  const x = setup();
  x.uploads.session.model = "nyxid/revoked";
  x.uploads.session.applyDraftModel = async () => { throw new Error("The selected model is unavailable."); };
  x.uploads.session.submit = async (submit) => submit({ conversationId: A, attachmentIds: [], attachments: [] });
  x.input().props.onChangeText("Keep this draft");
  x.harness.flush();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.events.some(([name]) => name === "send"), false);
  assert.equal(x.input().props.value, "Keep this draft");
  assert.equal(x.uploads.session.model, "nyxid/revoked");
  x.harness.unmount();
});

test("opening voice reuses a draft prepared earlier and applies its model before adoption", async () => {
  const x = setup();
  x.uploads.session.model = "nyxagent/research";
  x.uploads.session.ensureConversation = async () => A;
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.voice()?.props.thread, A);
  assert.deepEqual(x.events.filter(([name]) => name === "patch-draft-model" || name === "open"), [
    ["patch-draft-model", A, "nyxagent/research"],
    ["open", A, undefined],
  ]);
  assert.equal(x.drafts.length, 0);
  x.harness.unmount();
});

test("a batched reply commits through the matching saved bubble after the live row is gone", async () => {
  const x = setup(A);
  x.perf.enabled = true;
  x.input().props.onChangeText("Private prompt");
  x.harness.flush();
  x.button("Send message")!.props.onPress();
  await x.harness.settle();
  const sample = x.perf.sample!;
  const turnId = "fast-turn";
  const reply = { cursor: 1, turnId, blocks: [{ id: "b", index: 0, text: "Private answer" }] };
  sample.admitted();
  sample.observe({ event: "turn.status", turn_id: turnId, conversation_id: A, status: "running", cursor: 1 }, reply);
  sample.observe({ event: "block.delta", block_id: "b", text: "Private answer", cursor: 2 }, reply);
  sample.finish("completed");
  const row = (id: string, seq: number) => messageSchema.parse({
    id, seq, turn_id: id, role: "assistant", text: "Private answer", status: "completed",
    error_code: null, created_at: "2026-11-14T01:00:00Z", activities: [], attachments: [],
  });
  x.assistant.history!.messages = [row("older-turn", 1), row(turnId, 2)];
  x.harness.render();
  const list = x.all().find((node) => node.type === "FlatList")!;
  const body = (index: number) => elements(expand(list.props.renderItem({ item: list.props.data[index], index })))
    .find((node) => node.type === "ReplyBody")!.props;
  assert.equal(body(0).onTextCommit, undefined);
  assert.equal(x.perf.submitted.length, 0);
  assert.equal(body(1).text, "Private answer");
  const commit = body(1).onTextCommit;
  commit();
  commit();
  assert.equal(x.perf.submitted.length, 1);
  assert.equal(x.perf.submitted[0]?.outcome, "completed");
  assert.equal(typeof x.perf.submitted[0]?.first_text_commit_ms, "number");
  assert.equal(body(1).onTextCommit, undefined);
  x.harness.unmount();
});

test("transcript handlers forward frame boundaries while retaining scroll follow", () => {
  const view = setup("thread-a");
  assert.equal(view.nativePerf.frames.at(-1)?.focused, false);
  view.setTranscriptReady(true);
  assert.equal(view.nativePerf.frames.at(-1)?.focused, true);
  const list = view.all().find((element) => element.type === "FlatList")!;
  list.props.onScrollBeginDrag();
  list.props.onScrollEndDrag({});
  list.props.onMomentumScrollBegin();
  list.props.onMomentumScrollEnd({});
  assert.deepEqual(view.nativePerf.events, ["drag", "drag-end", "momentum", "momentum-end"]);
  assert.deepEqual(view.events.filter((event) => String(event[0]).startsWith("follow-")), [["follow-drag"], ["follow-drag-end"], ["follow-momentum"], ["follow-momentum-end"]]);
  view.setFocused(false);
  assert.equal(view.nativePerf.frames.at(-1)?.focused, false);
  view.harness.unmount();
});

test("the real assistant thread passes the decoded proposal and exact path to its approval control", async () => {
  const x = setup(A);
  const id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const reviewPath = `/assistant?c=${A}&card=${id}`;
  const page = historySchema.parse({
    conversation: { id: A, title: "Review", model: "nyxagent/chat", created_at: "2026-10-28", last_message_at: "2026-10-28", message_count: 0, active_turn: null },
    messages: [], before_seq: null, approvals: [{
      id, service_name: "Sheets", summary: "Write cells", expires_at: "2099-01-01", review_path: reviewPath,
      execution_proposal: { version: 4, state: "pending", supported: true, can_decide: true, review_path: reviewPath },
    }],
  });
  x.assistant.history!.approvals = page.approvals;
  x.harness.render();
  const approvalControl = () => elements(expand(x.all().find((node) => node.type === "FlatList")!.props.ListFooterComponent))
    .find((node) => node.type === "ApprovalCard")!;
  const card = approvalControl();
  assert.ok(card);
  assert.equal(card.props.approval, page.approvals[0]);
  assert.ok(card.props.approval?.execution_proposal);
  assert.equal(card.props.approval.execution_proposal.review_path, reviewPath);
  // Ordinary rows still use the normal chat decision callback.
  x.assistant.history!.approvals = [{ ...page.approvals[0]!, execution_proposal: undefined }];
  x.harness.render();
  const ordinary = approvalControl();
  await ordinary.props.onDecide(false);
  assert.deepEqual(x.events.at(-1), ["decideApproval", id, false]);
  x.harness.unmount();
});
