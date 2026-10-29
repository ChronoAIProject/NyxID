import { createAssistantDraftStore } from "../assistant/assistantDraftStore";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];

test("crew header shows member identity and narrows recipients through its picker without losing draft", () => {
  let options: any;
  const members = [
    { id: "bot", name: "nyxbot", display_name: "NyxBot", kind: "nyxbot" },
    {
      id: "research",
      name: "researcher",
      display_name: "Researcher",
      kind: "specialist",
    },
  ];
  const chat = {
    active: true,
    epoch: 1,
    online: true,
    sending: false,
    available: () => true,
    data: {
      group: {
        name: "Launch crew",
        members,
        lead_agent_id: "bot",
        default_recipient_ids: ["bot"],
        working_agent_ids: [] as string[],
      },
      messages: [],
      pending_actions: [] as any[],
    },
  };
  const h = hookHarness<E>({
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => false,
    },
    "../assistant/assistantDraftStore": {
      assistantDraftStore: createAssistantDraftStore(() => 1),
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
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
          "TextInput",
          "View",
        ].map((n) => [n, n]),
      ),
      StyleSheet: { create: (v: unknown) => v },
    },
    "lucide-react-native": {
      Check: "Check",
      Plus: "Plus",
      Settings2: "Settings",
      X: "X",
    },
    "../assistant/ChatIdentityHeader": {
      ChatHeaderFrame: "ChatHeaderFrame",
      ChatIdentity: "ChatIdentity",
    },
    "../../components/AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "Stack",
    },
    "../display/ChatLayoutContext": { useChatColumn: () => ({}) },
    "../../components/workspace/ScrollEdgeFades": {
      ScrollEdgeFades: "Fades",
      OverflowScrollView: "ScrollView",
      useScrollEdges: () => ({ edges: {} }),
    },
    "@tanstack/react-query": {
      useQuery: () => ({ data: { agents: members } }),
    },
    "../assistant/AssistantReplyBody": { AssistantReplyBody: "ReplyBody" },
    "react-native-markdown-display": {
      __esModule: true,
      default: "Markdown",
      MarkdownIt: () => ({}),
    },
    "expo-web-browser": {},
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Label", "Notice", "WorkspacePage"].map((n) => [
        n,
        n,
      ]),
    ),
    "../../lib/api/accountApi": {},
    "../../lib/api/agentsApi": {},
    "../../lib/api/groupsApi": {},
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
    "../../app/WorkspaceHeader": {
      WorkspaceHeaderVisibility: "Visibility",
      useWorkspaceBack: (p: any) => {
        options = p.options;
      },
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../assistant/AssistantPendingUploads": {
      AssistantPendingUploads: "Uploads",
    },
    "../assistant/AssistantMentionInput": { AssistantMentionInput: "Input" },
    "../assistant/ChatComposer": Object.fromEntries(
      [
        "ChatComposerFrame",
        "ChatComposerRow",
        "ComposerAttachButton",
        "ComposerSubmitButton",
      ].map((n) => [n, n]),
    ),
    "../assistant/AssistantActivityDisclosure": {},
    "../assistant/AssistantBanner": {
      AssistantConnectionBanner: "Connection",
      AssistantErrorNotice: "Error",
      JumpToLatest: "Jump",
    },
    "./GroupPendingCard": {},
    "./GroupMachineReceipts": {},
    "../assistant/AssistantMessageFooter": {
      AssistantMessageFooter: "Footer",
      AssistantMessageTime: "Time",
    },
    "../assistant/assistantSelectableMarkdown": {},
    "../assistant/useAssistantUploads": {
      useAssistantUploads: () => ({ files: [], session: { sending: false } }),
    },
    "../assistant/useComposerDictation": {
      useComposerDictation: () => ({ active: false }),
    },
    "../assistant/ComposerDictation": {
      ComposerDictationButton: "Dictation",
      ComposerDictationReview: "DictationReview",
    },
    "../assistant/useTranscriptScrollFollow": {
      useTranscriptScrollFollow: () => ({ listProps: {} }),
    },
    "./useGroupRuntime": {
      useGroupAvailability: () => ({ epoch: 1 }),
      groupKeys: () => [],
      useGroupChat: () => chat,
    },
  });
  const Screen = h.load(
    fileURLToPath(new URL("./GroupChatScreen.tsx", import.meta.url)),
  ).GroupChatScreen as any;
  h.mount(() => {
    const outer = Screen({
      groupId: "crew",
      onBack: () => {},
      onSettings: () => {},
    });
    return outer.type(outer.props);
  });
  const page = h.value();
  const list = nodes(page).find((n) => n.type === "FlatList")!;
  const viewport = nodes(page).find((n) =>
    [n.props.children].flat(Infinity).includes(list),
  )!;
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
  assert.equal(content.alignSelf, "center");
  assert.equal(list.props.scrollIndicatorInsets, undefined);
  const identity = () =>
    nodes(options.headerTitle()).find((n) => n.type === "ChatIdentity")!;
  assert.equal(identity().props.name, "Launch crew");
  assert.equal(identity().props.detail, "NyxBot, Researcher");
  assert.equal(identity().props.members.length, 2);
  const input = () => nodes(h.value()).find((n) => n.type === "Input")!;
  input().props.onChangeText("Keep this crew draft");
  h.flush();
  identity().props.onPress();
  h.flush();
  assert.ok(
    nodes(h.value()).some((n) => n.type === "Modal" && n.props.visible),
  );
  const choice = nodes(h.value()).find(
    (n) => n.props.accessibilityLabel === "Message Researcher",
  )!;
  choice.props.onPress();
  h.flush();
  assert.equal(input().props.value, "Keep this crew draft");
  assert.ok(
    nodes(h.value()).some(
      (n) => n.props.accessibilityLabel === "Remove recipient Researcher",
    ),
  );
  assert.ok(
    !nodes(h.value()).some(
      (n) => n.props.accessibilityLabel === "Remove recipient NyxBot",
    ),
  );
  chat.data.group.working_agent_ids = ["research"];
  h.render();
  assert.equal(identity().props.status, "Running");
  const working = nodes(h.value()).find((n) => n.type === "FlatList")!.props
    .ListFooterComponent;
  assert.equal(
    working.props.accessibilityLabel,
    "Agents working. Researcher is working…",
  );
  assert.equal(working.props.accessibilityLiveRegion, "polite");
  assert.ok(
    nodes(working).some(
      (n) => n.type === "AgentAvatar" && n.props.agent.id === "research",
    ),
  );
  assert.ok(nodes(working).some((n) => n.type === "ActivityIndicator"));
  chat.data.pending_actions = [
    { summary: "Needs confirmation", conversation_id: "chat" },
  ];
  h.render();
  assert.equal(identity().props.status, "Needs you");
  chat.data.group.working_agent_ids = [];
  h.render();
  assert.equal(
    nodes(h.value()).find((n) => n.type === "FlatList")!.props
      .ListFooterComponent,
    null,
  );
  h.unmount();
});
