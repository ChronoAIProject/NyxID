import assert from "node:assert/strict";
import test from "node:test";
import {
  activeWorkspaceChatTarget,
  workspaceChatNavigationAction,
  workspaceContextChatNavigationAction,
  type WorkspaceChatTarget,
} from "./workspaceChatNavigation";
import { assistantHomeParams } from "./assistantChatParams";
import { openWorkspaceSheetAction } from "./navigationHierarchy";
import {
  apply,
  backContent,
  content,
  fixture,
  pushContent,
  selected,
} from "./workspaceNavigationTestHarness";

const select = (
  root: ReturnType<typeof fixture>,
  target: WorkspaceChatTarget,
) => apply(root, workspaceChatNavigationAction(root, target));

test("sidebar chat selection removes obsolete pages while retaining mounted root/content/chat keys", () => {
  const caller = pushContent(pushContent(fixture(), "Agents"), "AgentDetail", {
    agentId: "old",
  });
  const opened = apply(caller, openWorkspaceSheetAction("AccountSettings"));
  const next = select(opened, {
    kind: "agent",
    agentId: "selected",
    conversationId: "selected-chat",
  });
  assert.deepEqual(
    next.routes.map((route) => route.name),
    ["Workspace"],
  );
  assert.equal(next.routes[0]!.key, caller.routes[0]!.key);
  assert.equal(content(next).key, content(caller).key);
  assert.equal(content(next).routes.length, 1);
  assert.equal(selected(content(next)).key, content(caller).routes[0]!.key);
  assert.deepEqual(selected(content(next)).params, {
    home: false,
    agentId: "selected",
    conversationId: "selected-chat",
    acknowledgementId: undefined,
    draft: false,
    openLatest: false,
  });
});

test("per-agent compose always starts a fresh empty normal chat for NyxBot and specialists", () => {
  for (const agentId of [undefined, "nyxbot", "specialist"]) {
    for (const initial of [
      fixture(),
      select(fixture(), { kind: "home" }),
      pushContent(fixture(), "AssistantContext", {
        agentId,
        conversationId: "caller",
      }),
    ]) {
      const next = select(initial, { kind: "agent", agentId });
      const params = selected(content(next)).params as any;
      assert.equal(params.home, false);
      assert.equal(params.agentId, agentId);
      assert.equal(params.conversationId, undefined);
      assert.equal(params.draft, true);
      assert.equal(params.openLatest, false);
      assert.match(params.draftId, /^draft-/);
      const again = select(next, { kind: "agent", agentId });
      assert.notEqual(
        (selected(content(again)).params as any).draftId,
        params.draftId,
      );
    }
  }
});

test("logo/Chat Home is explicit and never inherits an agent or thread draft", () => {
  const root = fixture();
  const next = select(root, { kind: "home" });
  assert.deepEqual(selected(content(next)).params, assistantHomeParams());
  assert.equal(selected(content(next)).key, selected(content(root)).key);
  assert.equal(
    activeWorkspaceChatTarget(selected(content(next)))?.kind,
    "home",
  );
  const compose = select(next, { kind: "agent" });
  assert.equal(
    activeWorkspaceChatTarget(selected(content(compose)))?.kind,
    "agent",
  );
});

test("crew selections reuse the exact current crew key and Back returns to the saved base transcript", () => {
  const caller = pushContent(fixture(), "GroupChat", { groupId: "crew" });
  const opened = pushContent(caller, "GroupEditor", { groupId: "crew" });
  const next = select(opened, { kind: "group", groupId: "crew" });
  assert.deepEqual(
    content(next).routes.map((route) => route.name),
    ["Assistant", "GroupChat"],
  );
  assert.equal(selected(content(next)).key, selected(content(caller)).key);
  assert.deepEqual(content(backContent(next)).routes, [
    content(caller).routes[0],
  ]);
  const other = select(next, { kind: "group", groupId: "other" });
  assert.notEqual(selected(content(other)).key, selected(content(next)).key);
  assert.deepEqual(content(other).routes[0], content(caller).routes[0]);
});

test("contextual chats push above their actual caller and retain its editor and draft", () => {
  for (const name of ["AgentDetail", "GroupEditor", "AssistantContext"]) {
    const caller = pushContent(fixture(), name, {
      agentId: "old",
      groupId: "crew",
      draftId: "unsent-draft",
    });
    const params = { agentId: "specialist", draft: true, draftId: "new-draft" };
    const next = apply(
      caller,
      workspaceContextChatNavigationAction(caller, params),
    );
    assert.deepEqual(content(next).routes.slice(0, -1), content(caller).routes);
    assert.deepEqual(selected(content(next)).params, params);
    assert.deepEqual(content(backContent(next)).routes, content(caller).routes);
    const another = apply(
      next,
      workspaceContextChatNavigationAction(next, {
        ...params,
        draftId: "another",
      }),
    );
    assert.notEqual(
      selected(content(another)).key,
      selected(content(next)).key,
    );
  }
});

test("a contextual chat requested by a mounted content screen targets that navigator", () => {
  const caller = fixture();
  const action = workspaceContextChatNavigationAction(content(caller), {
    agentId: "specialist",
  })!;
  assert.equal(action.target, content(caller).key);
  const next = apply(caller, action);
  assert.equal(selected(content(next)).name, "AssistantContext");
});

test("chat navigation waits for authentication and initializes an unmounted content stack", () => {
  assert.equal(
    workspaceChatNavigationAction(undefined, { kind: "agent" }),
    null,
  );
  assert.equal(workspaceContextChatNavigationAction(undefined, {}), null);
  const signedOut = {
    ...fixture(),
    routes: [{ name: "Auth", key: "auth" }],
    routeNames: ["Auth"],
  };
  assert.equal(
    workspaceChatNavigationAction(signedOut, { kind: "agent" }),
    null,
  );
  assert.equal(workspaceContextChatNavigationAction(signedOut, {}), null);
  const pending = {
    ...fixture(),
    routes: [{ name: "Workspace", key: "pending" }],
  };
  const next = select(pending, { kind: "agent", agentId: "specialist" });
  assert.equal((selected(content(next)).params as any).agentId, "specialist");
});

test("active chat targets preserve exact identities and ignore non-chat routes", () => {
  assert.deepEqual(
    activeWorkspaceChatTarget({ name: "Assistant", params: { home: true } }),
    { kind: "home" },
  );
  for (const params of [
    { home: true, agentId: "agent" },
    { home: true, conversationId: "chat" },
    { home: true, draft: true },
  ]) {
    assert.equal(
      activeWorkspaceChatTarget({ name: "Assistant", params })?.kind,
      "agent",
    );
  }
  for (const name of ["Assistant", "AssistantContext"]) {
    assert.deepEqual(
      activeWorkspaceChatTarget({
        name,
        params: {
          agentId: "agent",
          conversationId: "chat",
          acknowledgementId: "card",
        },
      }),
      { kind: "agent", agentId: "agent", conversationId: "chat" },
    );
    assert.deepEqual(
      activeWorkspaceChatTarget({ name }),
      name === "Assistant"
        ? { kind: "home" }
        : {
            kind: "agent",
            agentId: undefined,
            conversationId: undefined,
          },
    );
    assert.deepEqual(
      activeWorkspaceChatTarget({
        name,
        params: { agentId: 12, conversationId: "" },
      }),
      { kind: "agent", agentId: undefined, conversationId: undefined },
    );
  }
  assert.deepEqual(
    activeWorkspaceChatTarget({
      name: "GroupChat",
      params: { groupId: "group" },
    }),
    { kind: "group", groupId: "group" },
  );
  for (const route of [
    undefined,
    { name: "GroupChat" },
    { name: "Agents", params: { groupId: "group" } },
    { name: "AgentDetail", params: { agentId: "agent" } },
  ]) {
    assert.equal(activeWorkspaceChatTarget(route), undefined);
  }
});

test("opening an existing contextual conversation returns to its exact route instead of duplicating it", () => {
  const caller = pushContent(fixture(), "AssistantContext", {
    conversationId: "current",
    agentId: "agent",
  });
  const manager = pushContent(caller, "AgentDetail", { agentId: "agent" });
  const params = {
    conversationId: "current",
    agentId: "agent",
    acknowledgementId: "card",
  };
  const next = apply(
    manager,
    workspaceContextChatNavigationAction(manager, params),
  );
  assert.equal(content(next).routes.length, content(caller).routes.length);
  assert.equal(selected(content(next)).key, selected(content(caller)).key);
  assert.deepEqual(selected(content(next)).params, params);
});
