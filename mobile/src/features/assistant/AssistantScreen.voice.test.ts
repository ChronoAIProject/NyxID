import {
  createAssistantDraftStore,
  assistantDraftKey,
} from "./assistantDraftStore";
import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import type { AssistantParams } from "../../app/navigationTypes";
import type { AssistantUploadedMessage } from "./AssistantUploadSession";
import * as voiceSelection from "./voice/voiceSelection";
import {
  acknowledgementSchema,
  type AssistantAcknowledgement,
} from "./assistantTypes";

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
) {
  let onAdopt: ((id: string) => void) | undefined;
  let headerOptions: Record<string, any> = {};
  const headerModule = {};
  const draft = deferred<string>();
  const events: unknown[][] = [];
  const drafts: { agentId: string; signal: AbortSignal }[] = [];
  const featureQueries: { enabled: boolean }[] = [];
  let epoch = 7;
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
      messages: [],
      waiting: [],
      acknowledgements: [] as AssistantAcknowledgement[],
      approvals: [],
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
    conversations: [],
    inputAttachments: [],
    online: true,
    sending: false,
    loading: false,
    loadingOlder: false,
    recoveryRequired: false,
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
    ) => {
      events.push(["send", text, files]);
      return true;
    },
    setModel: (model: string) => events.push(["model", model]),
    stop: async () => {
      events.push(["stop"]);
    },
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
    files: [] as any[],
    session: {
      sending: false,
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
  const harness = hookHarness<Element>({
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => reducedMotion,
    },
    "./assistantDraftStore": {
      assistantDraftKey,
      assistantDraftStore: createAssistantDraftStore(() => epoch),
    },
    "./ChatIdentityHeader": headerModule,
    "react-native-safe-area-context": { SafeAreaView: "SafeAreaView" },
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
      Alert: { alert: () => {} },
      Keyboard: { dismiss: () => events.push(["dismissKeyboard"]) },
      StyleSheet: { create: (value: unknown) => value },
    },
    "@react-navigation/native": {
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
        "SquarePen",
        "ArrowUp",
        "Square",
        "X",
        "Mic",
      ].map((name) => [name, name]),
    ),
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: (input: { options: Record<string, any> }) => {
        headerOptions = input.options;
      },
      WorkspaceHeaderVisibility: "WorkspaceHeaderVisibility",
    },
    "../../app/navigationHierarchy": {},
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["WorkspacePage", "ActionButton", "Card", "Field", "Label", "Notice"].map(
        (name) => [name, name],
      ),
    ),
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { warningTone: {} } }),
    },
    "../../components/workspace/WorkspaceChatList": {
      WorkspaceChatList: (props: Record<string, any>) => ({
        type: "WorkspaceChatList",
        props: { ...props, children: [props.scrollHeader, props.scrollFooter] },
      }),
    },
    "./useAssistantGuidance": { useAssistantGuidance: () => guidance },
    "./useComposerConnection": { useComposerConnection: () => connection },
    "./ComposerConnectionPicker": {
      ComposerConnectionPicker: "ConnectionPicker",
    },
    "./NyxBotHome": { NyxBotHome: "NyxBotHome" },
    "./useAssistant": {
      useAssistant: (_id: unknown, _agent: unknown, adopt: typeof onAdopt) => {
        onAdopt = adopt;
        return assistant;
      },
    },
    "./useAssistantUploads": { useAssistantUploads: () => uploads },
    "./useComposerDictation": {
      useComposerDictation: () => ({ active: false, phase: "idle" }),
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
        following: true,
        listProps: {
          onLayout: (event: any) =>
            events.push(["follow-layout", event.nativeEvent.layout.height]),
          onContentSizeChange: (_width: number, height: number) =>
            events.push(["follow-content", height]),
          onScroll: (event: any) =>
            events.push(["follow-scroll", event.nativeEvent.contentOffset.y]),
          onScrollBeginDrag: () => events.push(["follow-drag"]),
        },
        follow: () => {},
        pause: () => {},
      }),
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
        renderActions?: (mention: null) => unknown;
      }) => ({
        type: "AssistantMentionInput",
        props: { ...props, children: props.renderActions?.(null) },
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
    "../../lib/env": { NYXID_FRONTEND_URL: "https://nyx.example" },
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
          drafts.push({ agentId, signal });
          return draft.promise;
        },
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
  });
  Object.assign(
    headerModule,
    harness.load(
      fileURLToPath(new URL("./ChatIdentityHeader.tsx", import.meta.url)),
    ),
  );
  const Screen = harness.load(
    fileURLToPath(new URL("./AssistantScreen.tsx", import.meta.url)),
  ).AssistantScreen as (input: typeof props) => Element;
  harness.mount(() => expand(Screen(props)));
  const all = () => elements(harness.value());
  const button = (name: string) =>
    all().find((element) => element.props.accessibilityLabel === name);
  const voice = () =>
    all().find((element) => element.type === "AssistantVoiceConversation");
  return {
    harness,
    assistant,
    uploads,
    connection,
    guidance,
    props,
    events,
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
    header: () => expand(headerOptions.headerTitle()),
    options: () => {
      elements(expand(headerOptions.headerRight()))
        .find((n) => n.props.accessibilityLabel === "Chat options")!
        .props.onPress();
      harness.flush();
    },
    compose: () => {
      const actions = expand(headerOptions.headerRight());
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
    setFeatures: (value: string[] | undefined) => {
      features = value;
      harness.render();
    },
  };
}

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

test("voice and transcript retain distinct sibling identities for both adopted drafts and saved chats", async () => {
  for (const initialId of [undefined, A]) {
    const x = setup(initialId);
    x.button("Open voice call")!.props.onPress();
    if (!initialId) x.draft.resolve(A);
    await x.harness.settle();
    const children = x.harness.value().props.children as Element[];
    const keyed = children.filter((child) => child?.key != null);
    assert.ok(keyed.some((child) => child.key === "transcript-column"));
    assert.ok(
      keyed.some((child) => child.type === "AssistantVoiceConversation"),
    );
    assert.equal(
      new Set(keyed.map((child) => child.key)).size,
      keyed.length,
      "a transcript and its voice surface share a conversation, but need unique sibling keys",
    );
    const transcript = x.all().find((element) => element.type === "FlatList")!;
    const originalTranscriptKey = transcript.key;
    x.voice()!.props.onClose();
    x.harness.flush();
    assert.equal(x.voice(), undefined);
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
    { role: "subagent" },
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

test("voice review returns to its exact saved chat and closing restores the entry button", async () => {
  const x = setup(A);
  x.button("Open voice call")!.props.onPress();
  await x.harness.settle();
  assert.equal(x.button("Open voice call"), undefined);
  x.voice()!.props.onUpdate();
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
  assert.equal(x.events.filter(([name]) => name === "recover").length, 1);
  x.voice()!.props.onClose();
  x.harness.flush();
  assert.equal(x.voice(), undefined);
  assert.ok(x.button("Open voice call"));
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

test("Choose agent preserves its hierarchy and dismisses before pushing History or Details into main content", () => {
  for (const [label, route] of [
    ["History", "AssistantHistory"],
    ["Details", "AssistantDetails"],
  ]) {
    const x = setup(A);
    x.chooseAgent();
    const hierarchy = x.all().find((n) => n.type === "WorkspaceChatList")!;
    assert.equal(hierarchy.props.mode, "picker");
    assert.equal(hierarchy.props.current.agentId, "agent-a");
    assert.equal(hierarchy.props.current.conversationId, A);
    const labels = x
      .all()
      .filter((n) => n.type === "Label")
      .flatMap((n) => n.props.children.flat(Infinity));
    assert.ok(labels.includes("This thread"));
    assert.ok(labels.includes("Saved chat"));
    assert.ok(labels.includes(12));
    const button = x
      .all()
      .find((n) => n.type === "ActionButton" && n.props.label === label)!;
    button.props.onPress();
    button.props.onPress();
    x.harness.flush();
    assert.equal(
      x.all().some((n) => n.type === "WorkspaceChatList"),
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
    .find((node) => node.type === "WorkspaceChatList")!
    .props.onSelect({ kind: "agent", agentId: "agent-a", conversationId: A });
  x.harness.flush();
  assert.equal(x.input().props.value, "Unsent review");
  assert.equal(
    x.events.some((event) => (event as unknown[])[0] === "open"),
    false,
  );
  x.chooseAgent();
  x.all()
    .find((node) => node.type === "WorkspaceChatList")!
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
    .find((node) => node.type === "WorkspaceChatList")!
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
    if (mode !== "nyxbot") {
      x.assistant.agent.kind = "specialist";
      x.assistant.agent.name = "researcher";
      x.assistant.agent.display_name = "Research team agent";
      x.props.contextual = mode === "contextual";
    }
    x.input().props.onChangeText("Unsent draft");
    x.harness.flush();
    const identity = elements(x.header()).find((n) => n.type === "Pressable")!;
    assert.match(
      identity.props.accessibilityLabel,
      /Chatting with .*Saved chat.*Choose agent/,
    );
    assert.ok(
      elements(x.header()).some(
        (n) => n.type === "AgentAvatar" && n.props.size === "xl",
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
    elements(draft.header()).find((n) => n.type === "Pressable")!.props
      .accessibilityLabel,
    /NyxBot, New chat\. Choose agent/,
  );
  draft.harness.unmount();
});

test("header needs-you status takes precedence over active-turn running status", () => {
  const x = setup(A, false);
  const conversation = x.assistant.history!.conversation;
  conversation.active_turn = { id: "turn", tools: [] };
  x.harness.render();
  assert.match(
    elements(x.header()).find((n) => n.type === "Pressable")!.props
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
    elements(x.header()).find((n) => n.type === "Pressable")!.props
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
      elements(x.header()).find((n) => n.type === "Pressable")!.props
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
      .find((n) => n.type === "WorkspaceChatList")!
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
    elements(x.header()).find((node) => node.type === "Pressable")!.props
      .accessibilityLabel,
    /NyxBot, Home\. Choose agent/,
  );
  assert.ok(x.all().some((node) => node.type === "NyxBotHome"));
  x.compose();
  x.harness.flush();
  assert.match(
    elements(x.header()).find((node) => node.type === "Pressable")!.props
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
