import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { agentSchema } from "../agents/agentContracts";
import { groupSchema } from "../groups/groupContracts";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { hookHarness } from "./assistantHookTestHarness";
import { chatAgentLabel } from "./assistantAgentContext";
import * as model from "./nyxBotHomeModel";

type Element = { type: unknown; props: Record<string, any> };
const CONVERSATION = `nyxa-${"a".repeat(32)}`;
const agent = (id: string, patch: object = {}) =>
  agentSchema.parse({
    id,
    name: id,
    description: "A real agent description",
    kind: "specialist",
    status: "idle",
    can_use: true,
    ...patch,
  });
const request = (id: string) => ({
  request_id: id,
  conversation_id: CONVERSATION,
  summary: `Approve ${id}`,
});

function setup() {
  const state = {
    agents: [
      agent("nyxbot", { kind: "nyxbot" }),
      agent("waiting", {
        pending_acknowledgements: 1,
        pending_requests: [request("decision-1")],
      }),
      agent("finished", {
        home_conversation_id: CONVERSATION,
        last_reply: {
          seq: 4,
          status: "completed",
          text: "The review is ready.",
          created_at: "2026-10-28T01:00:00Z",
        },
      }),
      agent("organization", {
        owner_kind: "org",
        owner_id: "org-a",
        owner_name: "Design team",
      }),
    ],
    crews: [
      groupSchema.parse({
        id: `nyxg-${"b".repeat(32)}`,
        name: "Launch crew",
        lead_agent_id: "nyxbot",
        members: [],
        created_at: "2026-10-28T00:00:00Z",
        working_agent_ids: ["waiting"],
      }),
    ],
    active: true,
    online: true,
    live: false,
    agentsPending: false,
    crewsPending: false,
    agentsError: false,
    crewsError: false,
  };
  const calls: unknown[][] = [];
  const queries = new Map<string, Record<string, any>>();
  const harness = hookHarness<Element>({
    "../../components/AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ScrollView: "ScrollView",
      View: "View",
      StyleSheet: { create: (value: unknown) => value, hairlineWidth: 1 },
    },
    "lucide-react-native": { Bot: "Bot", Users: "Users" },
    "@tanstack/react-query": {
      useQuery: (options: Record<string, any>) => {
        const name = options.queryKey.includes("agents")
          ? "agents"
          : options.queryKey.includes("groups")
            ? "crews"
            : "profile";
        queries.set(name, options);
        return {
          data:
            name === "agents"
              ? { agents: state.agents }
              : name === "crews"
                ? state.crews
                : { display_name: "Sam Review" },
          isPending:
            name === "agents"
              ? state.agentsPending
              : name === "crews" && state.crewsPending,
          isError:
            name === "agents"
              ? state.agentsError
              : name === "crews" && state.crewsError,
          refetch: () => calls.push(["refetch", name]),
        };
      },
    },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Card: "Card",
      Label: "Label",
      Notice: "Notice",
    },
    "../../components/SectionBadge": { SectionBadge: "SectionBadge" },
    "../../components/StatusBadge": { StatusBadge: "StatusBadge" },
    "../../lib/api/agentsApi": {
      agentsApi: {
        list: (signal: AbortSignal) => calls.push(["agents-api", signal]),
      },
    },
    "../../lib/api/groupsApi": {
      groupsApi: {
        list: (signal: AbortSignal) => calls.push(["crews-api", signal]),
      },
    },
    "../../lib/api/http": {
      getCurrentUserProfileRequest: (signal: AbortSignal) =>
        calls.push(["profile-api", signal]),
    },
    "../../lib/api/assistantLiveTransport": { livePollInterval },
    "../../lib/time": { formatRelativeTimeFromMs },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../theme/designTokens": { fonts: {} },
    "../groups/useGroupRuntime": {
      groupKeys: (epoch: number) => ["workspace", "groups", epoch],
      useGroupAvailability: () => ({
        epoch: 7,
        active: state.active,
        online: state.online,
        available: () => state.active && state.online,
      }),
    },
    "../groups/useGroupsLive": { useGroupsLive: () => state.live },
    "./assistantAgentContext": { chatAgentLabel },
    "./nyxBotHomeModel": model,
  });
  const Home = harness.load(
    fileURLToPath(new URL("./NyxBotHome.tsx", import.meta.url)),
  ).NyxBotHome as (props: unknown) => Element;
  const props = {
    onChat: (...args: unknown[]) => calls.push(["chat", ...args]),
    onOpenCrew: (id: string) => calls.push(["crew", id]),
    onNewAgent: () => calls.push(["new-agent"]),
    onNewCrew: () => calls.push(["new-crew"]),
  };
  harness.mount(() => Home(props));
  const elements = (node: Element): Element[] =>
    typeof node.type === "function"
      ? elements(node.type(node.props))
      : [
          node,
          ...[node.props.children]
            .flat(Infinity)
            .flatMap((child) =>
              child && typeof child === "object" && "props" in child
                ? elements(child)
                : [],
            ),
        ];
  const all = () => elements(harness.flush());
  const text = () =>
    all()
      .filter((node) => node.type === "Label")
      .flatMap((node) => [node.props.children].flat(Infinity))
      .filter((part) => typeof part === "string")
      .join(" ");
  const button = (name: string) => {
    const found = all().find(
      (node) =>
        node.type === "ActionButton" &&
        (node.props.accessibilityLabel ?? node.props.label) === name,
    );
    assert.ok(found, `Expected ${name}`);
    return found.props;
  };
  return { state, calls, queries, harness, all, text, button };
}

test("home renders real requests, completed replies, owner sections and crews with exact navigation", () => {
  const h = setup();
  assert.match(h.text(), /Sam/);
  for (const copy of [
    "While you were away",
    "Your agents",
    "Personal",
    "Design team",
    "Crews",
    "The review is ready.",
    "Launch crew",
  ])
    assert.ok(h.text().includes(copy), copy);
  assert.ok(
    h
      .all()
      .some(
        (node) =>
          node.type === "SectionBadge" &&
          node.props.label === "Needs a decision",
      ),
  );
  assert.ok(
    h
      .all()
      .some(
        (node) =>
          node.type === "SectionBadge" && node.props.label === "Replied",
      ),
  );
  h.button("Review waiting's request: Approve decision-1").onPress();
  h.button("Open finished's latest work").onPress();
  h.button("Chat with organization").onPress();
  h.button("Open crew Launch crew").onPress();
  h.button("New agent").onPress();
  h.button("New crew").onPress();
  assert.deepEqual(h.calls, [
    ["chat", "waiting", CONVERSATION, "decision-1"],
    ["chat", "finished", CONVERSATION, undefined],
    ["chat", "organization", undefined, undefined],
    ["crew", h.state.crews[0]!.id],
    ["new-agent"],
    ["new-crew"],
  ]);
  h.harness.unmount();
});

test("Show more reveals real additional work and Show less restores six rows", () => {
  const h = setup();
  h.state.agents = [
    agent("waiting", {
      pending_requests: Array.from({ length: 8 }, (_, index) =>
        request(`decision-${index}`),
      ),
    }),
  ];
  h.harness.render();
  const reviews = () =>
    h
      .all()
      .filter(
        (node) => node.type === "ActionButton" && node.props.label === "Review",
      );
  assert.equal(reviews().length, 6);
  h.button("Show more (2)").onPress();
  assert.equal(reviews().length, 8);
  h.button("Show less").onPress();
  assert.equal(reviews().length, 6);
  h.harness.unmount();
});

test("empty, loading and failed reads remain honest without synthetic rows", () => {
  const h = setup();
  h.state.agents = [];
  h.state.crews = [];
  h.harness.render();
  assert.match(h.text(), /Hi, I'm NyxBot/);
  assert.match(h.text(), /All caught up/);
  assert.match(h.text(), /No agents yet/);
  assert.equal(
    h
      .all()
      .filter(
        (node) => node.type === "ActionButton" && node.props.label === "Chat",
      ).length,
    0,
  );
  h.state.agentsPending = true;
  h.state.crewsPending = true;
  h.harness.render();
  assert.match(h.text(), /Loading your agents/);
  assert.doesNotMatch(h.text(), /Hi, I'm NyxBot|All caught up/);
  h.state.agentsPending = false;
  h.state.crewsPending = false;
  h.state.agentsError = true;
  h.state.crewsError = true;
  h.harness.render();
  assert.doesNotMatch(h.text(), /All caught up|No agents yet/);
  h.button("Retry agents").onPress();
  h.button("Retry crews").onPress();
  assert.deepEqual(h.calls, [
    ["refetch", "agents"],
    ["refetch", "crews"],
  ]);
  h.harness.unmount();
});

test("revocation, removed requests and inactive views fence retained callbacks and polling", () => {
  const h = setup();
  assert.equal(h.queries.get("agents")!.refetchInterval, 30_000);
  assert.equal(h.queries.get("agents")!.staleTime, 0);
  assert.equal(h.queries.get("crews")!.refetchInterval, 5000);
  const chat = h.button("Chat with organization").onPress;
  const review = h.button(
    "Review waiting's request: Approve decision-1",
  ).onPress;
  h.state.agents = h.state.agents.map((agent) => ({
    ...agent,
    can_use: agent.id !== "organization",
    pending_requests: [],
  }));
  h.harness.render();
  chat();
  review();
  assert.deepEqual(h.calls, []);
  const crew = h.button("Open crew Launch crew").onPress;
  h.state.active = false;
  h.state.online = false;
  h.harness.render();
  crew();
  assert.equal(h.button("New agent").disabled, true);
  assert.ok([...h.queries.values()].every((query) => query.enabled === false));
  assert.equal(h.queries.get("agents")!.refetchInterval, false);
  assert.deepEqual(h.calls, []);
  h.state.active = true;
  h.state.online = true;
  h.harness.render();
  assert.equal(h.queries.get("agents")!.enabled, true);
  assert.equal(h.queries.get("agents")!.refetchInterval, 30_000);
  assert.equal(h.queries.get("crews")!.refetchInterval, 5000);
  h.state.live = true;
  h.harness.render();
  assert.equal(h.queries.get("agents")!.refetchInterval, 30_000);
  h.harness.unmount();
});
