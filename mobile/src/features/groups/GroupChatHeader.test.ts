import { createAssistantDraftStore } from "../assistant/assistantDraftStore";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];

function setup({
  draftStore = createAssistantDraftStore(() => 1),
  missingResearch = false,
} = {}) {
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
  const posted: any[] = [];
  const chat: any = {
    active: true,
    epoch: 1,
    online: true,
    sending: false,
    available: () => true,
    post: async (body: any) => {
      posted.push(body);
      return true;
    },
    data: {
      group: {
        name: "Launch crew",
        members: missingResearch ? [members[0]] : members,
        lead_agent_id: "bot",
        default_recipient_ids: ["bot"],
        working_agent_ids: [] as string[],
      },
      messages: [],
      pending_actions: [] as any[],
      collaboration_decisions: [] as any[],
    },
  };
  const h = hookHarness<E>({
    "./CollaborationGroupControls": {
      CollaborationCards: "CollaborationCards",
      GroupAgentAccessRequest: "GroupAgentAccessRequest",
    },
    "../../lib/accessibility/useReducedMotion": {
      useReducedMotion: () => false,
    },
    "../assistant/assistantDraftStore": {
      assistantDraftStore: draftStore,
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
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
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
      useAssistantUploads: () => ({
        files: [],
        session: {
          sending: false,
          submit: (send: (files?: unknown) => Promise<boolean>) => send(),
        },
      }),
    },
    "../assistant/useComposerDictation": {
      useComposerDictation: () => ({ active: false }),
    },
    "../assistant/ComposerDictation": {
      ComposerDictationButton: "Dictation",
      ComposerDictationReview: "DictationReview",
    },
    "../assistant/useTranscriptScrollFollow": {
      useTranscriptScrollFollow: () => ({ listProps: {}, follow: () => {} }),
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
  const identity = () =>
    nodes(options.headerTitle()).find((n) => n.type === "ChatIdentity")!;
  const input = () => nodes(h.value()).find((n) => n.type === "Input")!;
  const submit = () =>
    nodes(input().props.renderActions()).find(
      (n) => n.type === "ComposerSubmitButton",
    )!;
  return { h, chat, posted, identity, input, submit };
}

test("crew header shows member identity and narrows recipients through its picker without losing draft", () => {
  const { h, chat, identity, input } = setup();
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
  assert.equal(identity().props.name, "Launch crew");
  assert.equal(identity().props.detail, "NyxBot, Researcher");
  assert.equal(identity().props.members.length, 2);
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
  chat.data.pending_actions = [];
  chat.data.collaboration_decisions = [{ can_decide: true }];
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

test("revoking or removing a mentioned shared agent blocks dispatch until an explicit replacement", async () => {
  for (const removed of [false, true]) {
    const { h, chat, posted, input, submit, identity } = setup();
    input().props.onChangeText("@researcher confidential question");
    h.flush();
    assert.equal(submit().props.disabled, false);
    if (removed) chat.data.group.members = [chat.data.group.members[0]];
    else chat.data.group.members[1].unavailable = true;
    h.render();
    assert.equal(submit().props.disabled, true);
    input().props.onChangeText(
      "@researcher confidential question with more detail",
    );
    h.flush();
    assert.equal(submit().props.disabled, true);
    submit().props.onPress();
    await h.settle();
    assert.equal(posted.length, 0);
    assert.equal(
      input().props.value,
      "@researcher confidential question with more detail",
    );
    assert.ok(
      nodes(h.value()).some((n) =>
        n.props.accessibilityLabel?.startsWith("Remove unavailable recipient "),
      ),
    );
    nodes(h.value())
      .find((n) =>
        n.props.accessibilityLabel?.startsWith("Remove unavailable recipient "),
      )!
      .props.onPress();
    h.flush();
    identity().props.onPress();
    h.flush();
    nodes(h.value())
      .find((n) => n.props.accessibilityLabel === "Message NyxBot")!
      .props.onPress();
    h.flush();
    assert.equal(submit().props.disabled, false);
    submit().props.onPress();
    await h.settle();
    assert.equal(posted.length, 1);
    assert.deepEqual(Array.from(posted[0].recipient_agent_ids), ["bot"]);
    h.unmount();
  }
});

test("UUID mentions and default recipients are sent as exact IDs, including a roster change at dispatch", async () => {
  for (const explicitMention of [false, true]) {
    const { h, chat, posted, input, submit } = setup();
    const id = "1785c70a-9a5b-41f8-b73e-6ff523cdd98e";
    chat.data.group.members[1].id = id;
    h.render();
    input().props.onChangeText(explicitMention ? `@${id} hello` : "hello");
    h.flush();
    submit().props.onPress();
    chat.data.group.default_recipient_ids = [id];
    await h.settle();
    assert.equal(posted.length, 1);
    assert.deepEqual(Array.from(posted[0].recipient_agent_ids), [
      explicitMention ? id : "bot",
    ]);
    h.unmount();
  }
});

test("reopening a retained draft keeps a removed agent's old handle unavailable", async () => {
  const draftStore = createAssistantDraftStore(() => 1);
  const original = setup({ draftStore });
  original.input().props.onChangeText("@researcher confidential question");
  original.h.flush();
  original.h.unmount();
  const reopened = setup({ draftStore, missingResearch: true });
  assert.equal(reopened.submit().props.disabled, true);
  reopened
    .input()
    .props.onChangeText("@researcher confidential question with more detail");
  reopened.h.flush();
  assert.equal(reopened.submit().props.disabled, true);
  reopened.submit().props.onPress();
  await reopened.h.settle();
  assert.equal(reopened.posted.length, 0);
  reopened.input().props.onChangeText("New question without the old mention");
  reopened.h.flush();
  assert.equal(reopened.submit().props.disabled, false);
  reopened.h.unmount();
});

test("renaming a mentioned agent and reusing its old handle never changes the draft target", async () => {
  for (const unavailable of [false, true]) {
    const { h, chat, posted, input, submit } = setup();
    input().props.onChangeText("@researcher confidential question");
    h.flush();
    chat.data.group.members = [
      chat.data.group.members[0],
      { ...chat.data.group.members[1], name: "analyst", unavailable },
      { id: "replacement", name: "researcher", kind: "specialist" },
    ];
    h.render();
    input().props.onChangeText(
      "@researcher confidential question with more detail",
    );
    h.flush();
    assert.equal(submit().props.disabled, unavailable);
    submit().props.onPress();
    await h.settle();
    assert.equal(posted.length, unavailable ? 0 : 1);
    if (!unavailable)
      assert.deepEqual(Array.from(posted[0].recipient_agent_ids), ["research"]);
    h.unmount();
  }
});

test("editing away an old handle releases its binding even while another mention keeps the same agent", async () => {
  const { h, chat, posted, input, submit } = setup();
  input().props.onChangeText("@researcher confidential question");
  h.flush();
  chat.data.group.members = [
    chat.data.group.members[0],
    { ...chat.data.group.members[1], name: "analyst" },
    { id: "replacement", name: "researcher", kind: "specialist" },
  ];
  h.render();
  input().props.onChangeText("@analyst different question");
  h.flush();
  input().props.onChangeText("@analyst and @researcher new question");
  h.flush();
  assert.equal(submit().props.disabled, false);
  submit().props.onPress();
  await h.settle();
  assert.deepEqual(Array.from(posted[0].recipient_agent_ids), [
    "research",
    "replacement",
  ]);
  h.unmount();
});

test("a definite refusal restores mention bindings after optimistic clear and a missing recipient", async () => {
  const draftStore = createAssistantDraftStore(() => 1);
  const { h, chat, posted, input, submit } = setup({ draftStore });
  const refusal = deferred<boolean>();
  chat.post = async (body: any) => {
    posted.push(body);
    return refusal.promise;
  };
  input().props.onChangeText("@researcher confidential question");
  h.flush();
  submit().props.onPress();
  h.flush();
  assert.equal(input().props.value, "");
  assert.equal(posted.length, 1);
  assert.deepEqual(Array.from(posted[0].recipient_agent_ids), ["research"]);
  chat.data.group.members = [chat.data.group.members[0]];
  h.render();
  refusal.resolve(false);
  await h.settle();
  assert.equal(input().props.value, "@researcher confidential question");
  assert.equal(submit().props.disabled, true);
  input().props.onChangeText(
    "@researcher confidential question with more detail",
  );
  h.flush();
  submit().props.onPress();
  await h.settle();
  assert.equal(posted.length, 1);
  h.unmount();
  const reopened = setup({ draftStore, missingResearch: true });
  assert.equal(reopened.submit().props.disabled, true);
  reopened.submit().props.onPress();
  await reopened.h.settle();
  assert.equal(reopened.posted.length, 0);
  reopened.h.unmount();
});

const collaboration = {
  version: 1,
  creator_user_id: "creator",
  membership_revision: 1,
  history_disclosure: "Full history",
  can_manage: false,
  your_request_ids: [],
  transfer_to_user_id: null,
  guest_access: [],
  proposals: [],
  invites: [],
};

test("a collaboration participant with no usable agent can still message people", async () => {
  const { h, chat, posted, input, submit } = setup();
  chat.data.group.collaboration = collaboration;
  chat.data.group.default_recipient_ids = [];
  for (const member of chat.data.group.members) {
    member.unavailable = true;
    member.unavailable_reason = "not_shared_with_you";
    member.owner_name = "Ada";
  }
  h.render();
  const text = () =>
    nodes(h.value())
      .filter((n) => n.type === "Label")
      .map((n) => [n.props.children].flat().join(""))
      .join(" | ");
  assert.match(text(), /no agent reply/);
  assert.doesNotMatch(text(), /read-only/);
  assert.equal(input().props.editable, true);
  input().props.onChangeText("hello everyone");
  h.flush();
  assert.equal(submit().props.disabled, false);
  submit().props.onPress();
  await h.settle();
  assert.equal(posted.length, 1);
  assert.deepEqual(Array.from(posted[0].recipient_agent_ids), []);
  h.unmount();
});

test("removing every recipient in a collaboration group sends a people-only message", async () => {
  const { h, chat, posted, input, submit } = setup();
  chat.data.group.collaboration = collaboration;
  h.render();
  nodes(h.value())
    .find((n) => n.props.accessibilityLabel === "Remove recipient NyxBot")!
    .props.onPress();
  h.flush();
  input().props.onChangeText("just people");
  h.flush();
  submit().props.onPress();
  await h.settle();
  assert.deepEqual(Array.from(posted[0].recipient_agent_ids), []);
  h.unmount();
});

test("a legacy crew still requires an agent recipient", () => {
  const { h, input, submit } = setup();
  nodes(h.value())
    .find((n) => n.props.accessibilityLabel === "Remove recipient NyxBot")!
    .props.onPress();
  h.flush();
  input().props.onChangeText("hello");
  h.flush();
  assert.equal(submit().props.disabled, true);
  h.unmount();
});

test("a crew's first load shows a transcript skeleton", () => {
  const { h, chat } = setup();
  chat.isPending = true;
  h.render();
  const list = nodes(h.value()).find((n) => n.type === "FlatList")!;
  assert.equal(list.props.ListEmptyComponent.type, "SkeletonRows");
  assert.equal(
    list.props.ListEmptyComponent.props.accessibilityLabel,
    "Loading crew",
  );
  h.unmount();
});
