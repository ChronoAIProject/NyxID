import { assistantHomeParams } from "./assistantChatParams";
import assert from "node:assert/strict";
import test from "node:test";
import { getStateFromPath } from "@react-navigation/core";
import { navigationLinkingConfig } from "./navigationLinking";
import { workspaceStateFromPath } from "./workspaceCanonicalLinks";
import { dismissWorkspaceSheetAction } from "./navigationHierarchy";
import { hydratedTestRouter } from "./routerTestHarness";
import { StackRouter } from "@react-navigation/routers";

const C = `nyxa-${"a".repeat(32)}`;
const G = `nyxg-${"b".repeat(32)}`;
const parse = (path: string) =>
  workspaceStateFromPath(path, (next) =>
    getStateFromPath(next, navigationLinkingConfig),
  );

test("web billing panel deep links open the read-only drawer over the addressed chat", () => {
  const state = parse(
    `assistant?c=${C}&agent=agent-one&panel=billing&panelTab=usage&panelPeriod=7d&panelAction=topup&panelService=provider`,
  )!;
  assert.deepEqual(
    state.routes.map((route) => route.name),
    ["Workspace", "WorkspaceSheet"],
  );
  assert.deepEqual(state.routes[0]!.state!.routes[0]!.params, {
    conversationId: C,
    agentId: "agent-one",
    draft: false,
    openLatest: false,
  });
  const sheet = state.routes[1]!.state!;
  assert.equal(sheet.routes[0]!.name, "AccountSettings");
  assert.equal(sheet.routes.at(-1)!.name, "UsageCredits");
  assert.deepEqual(sheet.routes.at(-1)!.params, { period: "7d" });
  const router = hydratedTestRouter(
    StackRouter({ initialRouteName: "Workspace" }),
  );
  const options = {
    routeNames: Object.keys(navigationLinkingConfig.screens!),
    routeParamList: {},
    routeGetIdList: {},
  };
  const hydrated = router.getRehydratedState(state, options);
  const action = dismissWorkspaceSheetAction(hydrated)!;
  const closed = router.getStateForAction(hydrated, action, options)!;
  assert.equal(closed.routes.length, 1);
  assert.deepEqual(closed.routes[0]!.state!.routes[0]!.params, {
    conversationId: C,
    agentId: "agent-one",
    draft: false,
    openLatest: false,
  });
});

test("billing cold entry retains a crew, selected-agent draft or latest thread", () => {
  const crew = parse(`assistant?g=${G}&panel=billing`)!;
  assert.deepEqual(
    crew.routes.map((route) => route.name),
    ["Workspace", "WorkspaceSheet"],
  );
  assert.deepEqual(crew.routes[0]!.state!.routes[1]!.params, { groupId: G });
  for (const draft of [true, false]) {
    const state = parse(
      `assistant?agent=agent-one&draft=${draft}&panel=billing`,
    )!;
    assert.deepEqual(state.routes[0]!.state!.routes[0]!.params, {
      agentId: "agent-one",
      draft,
      openLatest: !draft,
    });
  }
  const cold = parse("/assistant?panel=billing&panelPeriod=invalid")!;
  assert.equal(cold.routes[0]!.state!.routes[0]!.name, "Assistant");
  assert.equal(cold.routes.at(-1)!.state!.routes.at(-1)!.params, undefined);
});

test("internal billing links validate period, preserve owner, and unrelated links keep their existing parser", () => {
  const internal = parse("account/usage?period=7d&ownerId=org-one")!;
  assert.deepEqual(internal.routes.at(-1)!.state!.routes.at(-1)!.params, {
    period: "7d",
    ownerId: "org-one",
  });
  assert.deepEqual(
    parse("account/usage?period=invalid")!.routes.at(-1)!.state!.routes.at(-1)!
      .params,
    { period: "30d" },
  );
  assert.equal(parse("assistant?panel=settings"), undefined);
  assert.equal(parse("assistant?panel=billing&panel=billing"), undefined);
  assert.equal(
    parse("assistant/machines?tab=logins")!.routes.at(-1)!.state!.routes.at(-1)!
      .name,
    "Machines",
  );
});

test("canonical /assistant without a thread explicitly selects NyxBot home", () => {
  const state = parse("/assistant")!;
  assert.equal(state.routes.length, 1);
  const home = state.routes[0]!.state!.routes[0]!;
  assert.equal(home.name, "Assistant");
  assert.deepEqual(home.params, assistantHomeParams());
  const draft = parse("/assistant?draft=true")!.routes[0]!.state!.routes[0]!
    .params as any;
  assert.equal(draft.home, false);
  assert.equal(draft.draft, true);
  assert.ok(draft.draftId);
});

test("web chat links and the push deeplink open their thread; the notifications panel stacks the feed above it", () => {
  const chat = parse(`/assistant?c=${C}`)!;
  assert.deepEqual(
    chat.routes.map((route) => route.name),
    ["Workspace"],
  );
  assert.deepEqual(chat.routes[0]!.state!.routes.map((route) => route.name), ["Assistant"]);
  assert.deepEqual(chat.routes[0]!.state!.routes[0]!.params, {
    home: false,
    conversationId: C,
    draft: false,
    openLatest: false,
  });
  const panel = parse(`https://app.nyxid.test/assistant?c=${C}&agent=agent-one&panel=notifications`)!;
  const pages = panel.routes[0]!.state!;
  assert.deepEqual(pages.routes.map((route) => route.name), ["Assistant", "Notifications"]);
  assert.equal(pages.index, 1);
  assert.deepEqual(pages.routes[0]!.params, {
    home: false,
    conversationId: C,
    agentId: "agent-one",
    draft: false,
    openLatest: false,
  });
  const crew = parse(`/assistant?g=${G}&panel=notifications`)!.routes[0]!.state!;
  assert.deepEqual(crew.routes.map((route) => route.name), ["Assistant", "GroupChat", "Notifications"]);
  assert.deepEqual(crew.routes[1]!.params, { groupId: G });
  const cold = parse("/assistant?panel=notifications")!.routes[0]!.state!;
  assert.deepEqual(cold.routes.map((route) => route.name), ["Assistant", "Notifications"]);
  assert.deepEqual(cold.routes[0]!.params, assistantHomeParams());
  const agent = parse("/assistant?agent=agent-two")!.routes[0]!.state!.routes[0]!;
  assert.deepEqual(agent.params, { home: false, agentId: "agent-two", draft: false, openLatest: true });
  for (const path of ["/assistant/notifications", "notifications"]) {
    const direct = parse(path)!.routes[0]!.state!;
    assert.equal(direct.routes.at(-1)!.name, "Notifications", path);
    assert.equal(direct.routes[0]!.name, "Assistant", path);
  }
  assert.equal(parse("/assistant?c=not-a-thread&panel=notifications"), undefined, "malformed ids fall back to the ordinary parser");
  assert.equal(parse("/assistant?c=nyxa-short"), undefined);
  assert.equal(parse(`/assistant?c=${C}&panel=notifications&panel=billing`), undefined, "repeated panels are not a destination");
});
