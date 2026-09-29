import { describe, expect, it } from "vitest";
import {
  assistantAgentCreateSchema,
  assistantAgentDetailSchema,
  assistantAgentListSchema,
  assistantAgentProfileSchema,
  assistantGroupFormSchema,
  assistantGroupMessagesSchema,
  assistantGroupPostedSchema,
  nyxAgentAcknowledgementSchema,
  nyxAgentChannelConnectSchema,
  nyxAgentChannelListSchema,
  nyxAgentConversationSchema,
  nyxAgentHistorySchema,
  nyxAgentMessageSchema,
  nyxAgentSettingsFormSchema,
} from "./assistant-nyxagent";

const orchestrator = `nyxa-${"a".repeat(32)}`;
const specialistThread = `nyxa-${"b".repeat(32)}`;
const at = "2026-09-28T00:00:00Z";

const legacyConversation = {
  id: orchestrator,
  title: "Chat",
  model: "nyxagent/chat",
  access_mode: "ask",
  created_at: at,
  last_message_at: at,
  message_count: 2,
  pending_acknowledgements: 0,
  active_turn: null,
  context_reset_at: null,
};

describe("NyxBot conversation schema", () => {
  it("reads rows from servers before agents existed as unattributed NyxBot threads", () => {
    const row = nyxAgentConversationSchema.parse(legacyConversation);
    expect(row).toMatchObject({
      role: "orchestrator",
      agent: null,
      pending_events: 0,
      channel: null,
    });
    // The retired Ask/Full field is never surfaced.
    expect(row).not.toHaveProperty("access_mode");
  });

  it("parses the owning agent and channel origin, dropping the removed team fields", () => {
    const row = nyxAgentConversationSchema.parse({
      ...legacyConversation,
      id: specialistThread,
      access_mode: "full",
      role: "subagent",
      agent: { id: "agent-1", kind: "specialist", name: "researcher", destroyed: true },
      pending_events: 2,
      channel: { platform: "telegram" },
      team_id: orchestrator,
      members: [],
    });
    expect(row.agent).toEqual({
      id: "agent-1",
      kind: "specialist",
      name: "researcher",
      destroyed: true,
    });
    expect(row.pending_events).toBe(2);
    expect(row.channel).toEqual({ platform: "telegram" });
    expect(row).not.toHaveProperty("team_id");
    expect(row).not.toHaveProperty("members");
    expect(() =>
      nyxAgentConversationSchema.parse({
        ...legacyConversation,
        agent: { id: "a", kind: "robot", name: "x", destroyed: false },
      }),
    ).toThrow();
  });
});

describe("NyxBot message and card schemas", () => {
  const message = {
    id: "m",
    seq: 1,
    turn_id: "t",
    text: "Hello",
    status: "completed",
    error_code: null,
    created_at: at,
  };

  it.each(["user", "assistant", "orchestrator", "event"])("accepts the %s role", (role) => {
    expect(nyxAgentMessageSchema.parse({ ...message, role }).role).toBe(role);
  });

  it("rejects unknown roles", () => {
    expect(() => nyxAgentMessageSchema.parse({ ...message, role: "system" })).toThrow();
  });

  it("defaults acknowledgements to the user as decider and reads orchestrator routing", () => {
    const base = {
      id: "12345678-1234-4123-8123-123456789012",
      kind: "service",
      status: "allowed",
      summary: "Use GitHub",
      service_slug: "github",
      service_name: "GitHub",
      tool_name: null,
      created_at: at,
      decided_at: at,
      expires_at: at,
    };
    expect(nyxAgentAcknowledgementSchema.parse(base)).toMatchObject({
      decider: "user",
      decided_by: null,
      reason: null,
    });
    expect(
      nyxAgentAcknowledgementSchema.parse({
        ...base,
        decider: "orchestrator",
        decided_by: "orchestrator",
        reason: "The user asked for repository triage",
      }),
    ).toMatchObject({ decider: "orchestrator", decided_by: "orchestrator" });
  });

  it("parses a history with team members", () => {
    const page = nyxAgentHistorySchema.parse({
      conversation: { ...legacyConversation, members: [] },
      messages: [{ ...message, role: "event" }],
      before_seq: null,
    });
    expect(page.messages[0]?.role).toBe("event");
    expect(page.acknowledgements).toEqual([]);
  });
});

describe("NyxBot team, settings and channel schemas", () => {
  const limits = {
    skip_destructive_confirmation: false,
    max_live_subagents: 8,
    max_concurrent_subagent_turns: 3,
    max_live_subagents_limit: 32,
    max_concurrent_subagent_turns_limit: 8,
    max_group_handoffs: 6,
    max_group_handoffs_per_hour: 60,
    max_group_handoffs_limit: 24,
    max_group_handoffs_per_hour_limit: 600,
  };

  const agent = {
    id: "agent-1",
    kind: "specialist",
    name: "researcher",
    description: "Finds urgent issues",
    specialty: null,
    created_by: "nyxbot",
    status: "idle",
    services: ["github"],
    account_read: true,
    pending_requests: [
      {
        request_id: "r",
        agent: "researcher",
        agent_id: "agent-1",
        conversation_id: specialistThread,
        kind: "service",
        service_slug: "slack",
        summary: "Use Slack",
        requested_by: "Post the digest",
        expires_at: at,
      },
    ],
    last_reply: { seq: 4, status: "completed", text: "Done", created_at: at },
    home_conversation_id: specialistThread,
    memory_count: 2,
    created_at: at,
    last_active_at: at,
    destroyed_at: null,
    pending_acknowledgements: 1,
    channels: [{ id: "c", platform: "telegram", bot_label: "Home bot", status: "active" }],
  };

  it("parses the agents list with its limits", () => {
    const list = assistantAgentListSchema.parse({ agents: [agent], limits });
    expect(list.agents[0]).toMatchObject({
      kind: "specialist",
      created_by: "nyxbot",
      services: ["github"],
      memory_count: 2,
    });
    expect(list.agents[0]?.pending_requests[0]?.conversation_id).toBe(specialistThread);
    expect(list.agents[0]?.channels[0]?.platform).toBe("telegram");
    expect(list.limits.max_live_subagents_limit).toBe(32);
  });

  it("parses one agent with memory and threads", () => {
    const detail = assistantAgentDetailSchema.parse({
      agent,
      memory: [{ id: "n", text: "Weekly digests on Mondays", created_at: at, updated_at: at }],
      threads: [
        { id: specialistThread, title: "researcher", last_message_at: at, channel: null, running: true },
      ],
    });
    expect(detail.memory[0]?.text).toBe("Weekly digests on Mondays");
    expect(detail.threads[0]?.running).toBe(true);
  });

  it("validates new agents like the server", () => {
    const valid = { name: "researcher", description: "Finds issues", services: [], account_read: false };
    expect(assistantAgentCreateSchema.safeParse(valid).success).toBe(true);
    expect(assistantAgentCreateSchema.safeParse({ ...valid, name: "Researcher" }).success).toBe(false);
    expect(assistantAgentCreateSchema.safeParse({ ...valid, name: "-x" }).success).toBe(false);
    expect(assistantAgentCreateSchema.safeParse({ ...valid, name: "a".repeat(33) }).success).toBe(false);
    expect(assistantAgentCreateSchema.safeParse({ ...valid, description: " " }).success).toBe(false);
    // NyxBot keeps its name and its persona notes are optional.
    const style = { display_name: "", persona: "" };
    expect(
      assistantAgentProfileSchema("nyxbot").safeParse({ name: "NyxBot", description: "", ...style })
        .success,
    ).toBe(true);
    expect(
      assistantAgentProfileSchema("specialist").safeParse({
        name: "writer",
        description: "",
        ...style,
      }).success,
    ).toBe(false);
  });

  it("reads which agent a channel bot reaches (null is NyxBot)", () => {
    const rows = nyxAgentChannelListSchema.parse({
      channel_agents: [
        {
          id: "c",
          channel_bot_id: "bot",
          platform: "telegram",
          bot_label: "Bot",
          transport: "gateway",
          status: "active",
          owner_linked: true,
          created_at: at,
        },
      ],
    }).channel_agents;
    expect(rows[0]?.agent_id).toBeNull();
  });

  it("validates settings form values against the server limits", () => {
    const schema = nyxAgentSettingsFormSchema(limits);
    const valid = {
      confirm_destructive: true,
      max_live_subagents: 0,
      max_concurrent_subagent_turns: 1,
      max_group_handoffs: 0,
      max_group_handoffs_per_hour: 0,
    };
    expect(schema.safeParse(valid).success).toBe(true);
    expect(schema.safeParse({ ...valid, max_group_handoffs: 24 }).success).toBe(true);
    expect(schema.safeParse({ ...valid, max_group_handoffs: 25 }).success).toBe(false);
    expect(schema.safeParse({ ...valid, max_group_handoffs_per_hour: 601 }).success).toBe(false);
    expect(schema.safeParse({ ...valid, max_live_subagents: 33 }).success).toBe(false);
    expect(schema.safeParse({ ...valid, max_concurrent_subagent_turns: 0 }).success).toBe(false);
    expect(schema.safeParse({ ...valid, max_concurrent_subagent_turns: 9 }).success).toBe(false);
    expect(schema.safeParse({ ...valid, max_live_subagents: 2.5 }).success).toBe(false);
    expect(schema.safeParse({ ...valid, max_live_subagents: Number.NaN }).success).toBe(false);
  });

  it("keeps only https owner links", () => {
    const channel_agent = {
      id: "c",
      channel_bot_id: "bot",
      platform: "telegram",
      bot_label: "Bot",
      bot_username: "bot",
      transport: "gateway",
      status: "active",
      last_error: null,
      owner_linked: false,
      created_at: at,
    };
    const link = { code: "nyxlink_abc", expires_at: at, instructions: "Press Start" };
    expect(
      nyxAgentChannelConnectSchema.parse({
        channel_agent,
        link: { ...link, url: "https://t.me/bot?start=nyxlink_abc" },
      }).link.url,
    ).toBe("https://t.me/bot?start=nyxlink_abc");
    expect(
      nyxAgentChannelConnectSchema.parse({
        channel_agent,
        link: { ...link, url: "javascript:alert(1)" },
      }).link.url,
    ).toBeNull();
    expect(
      nyxAgentChannelConnectSchema.parse({ channel_agent, link: { ...link, url: null } }).link.url,
    ).toBeNull();
  });
});

describe("group schemas", () => {
  const group = {
    id: "nyxg-1",
    name: "Launch crew",
    members: [
      { id: "a1", name: "NyxBot", kind: "nyxbot", destroyed: false, working: false },
      { id: "a2", name: "researcher", kind: "specialist", destroyed: false, working: true },
    ],
    lead_agent_id: "a1",
    working_agent_ids: ["a2"],
    message_count: 2,
    last_message_at: at,
    created_at: at,
  };

  it("parses a transcript page with user, agent and notice messages", () => {
    const page = assistantGroupMessagesSchema.parse({
      group,
      messages: [
        { id: "m1", seq: 1, role: "notice", agent: null, text: "Started", created_at: at },
        { id: "m2", seq: 2, role: "user", text: "@researcher go", created_at: at },
        {
          id: "m3",
          seq: 3,
          role: "agent",
          agent: { id: "a2", name: "researcher", kind: "specialist" },
          text: "On it",
          created_at: at,
        },
      ],
      before_seq: null,
    });
    expect(page.group.members[1]).toMatchObject({ name: "researcher", working: true });
    expect(page.messages.map((message) => message.role)).toEqual(["notice", "user", "agent"]);
    expect(page.messages[1]?.agent).toBeNull();
    expect(() =>
      assistantGroupMessagesSchema.parse({
        group,
        messages: [{ id: "m", seq: 1, role: "assistant", text: "", created_at: at }],
        before_seq: null,
      }),
    ).toThrow();
  });

  it("parses the accepted post response", () => {
    expect(
      assistantGroupPostedSchema.parse({
        message: { id: "m", seq: 4, role: "user", agent: null, text: "hi", created_at: at },
        addressed_agent_ids: ["a1"],
      }).addressed_agent_ids,
    ).toEqual(["a1"]);
  });

  it("validates the create/settings form: a name and 1 to 8 agents", () => {
    expect(
      assistantGroupFormSchema.safeParse({ name: "  Crew  ", member_agent_ids: ["a"] }).data,
    ).toEqual({ name: "Crew", member_agent_ids: ["a"] });
    expect(assistantGroupFormSchema.safeParse({ name: " ", member_agent_ids: ["a"] }).success).toBe(false);
    expect(
      assistantGroupFormSchema.safeParse({ name: "x".repeat(61), member_agent_ids: ["a"] }).success,
    ).toBe(false);
    expect(assistantGroupFormSchema.safeParse({ name: "Crew", member_agent_ids: [] }).success).toBe(false);
    expect(
      assistantGroupFormSchema.safeParse({
        name: "Crew",
        member_agent_ids: Array.from({ length: 9 }, (_, index) => String(index)),
      }).success,
    ).toBe(false);
  });
});

describe("agent display name and persona", () => {
  const base = {
    id: "a1",
    kind: "specialist",
    name: "writer",
    status: "idle",
    created_at: at,
    last_active_at: at,
  };

  it("parses them, and treats older payloads as having none", () => {
    const list = assistantAgentListSchema.parse({
      agents: [base, { ...base, id: "a2", display_name: "Luna", persona: "Warm." }],
      limits: {
        skip_destructive_confirmation: false,
        max_live_subagents: 8,
        max_concurrent_subagent_turns: 3,
        max_live_subagents_limit: 32,
        max_concurrent_subagent_turns_limit: 8,
      },
    });
    expect(list.agents.map((agent) => [agent.display_name, agent.persona])).toEqual([
      [null, null],
      ["Luna", "Warm."],
    ]);
  });

  it("bounds them on create and edit; empty means none", () => {
    const create = {
      name: "writer",
      description: "Drafts",
      services: [],
      account_read: false,
      display_name: "",
      persona: "",
    };
    expect(assistantAgentCreateSchema.safeParse(create).success).toBe(true);
    expect(
      assistantAgentCreateSchema.safeParse({ ...create, display_name: "x".repeat(41) }).success,
    ).toBe(false);
    expect(
      assistantAgentCreateSchema.safeParse({ ...create, persona: "x".repeat(2001) }).success,
    ).toBe(false);
    const nyxbot = assistantAgentProfileSchema("nyxbot");
    expect(
      nyxbot.safeParse({ name: "NyxBot", display_name: " Nyx ", description: "", persona: "" }).data,
    ).toEqual({ name: "NyxBot", display_name: "Nyx", description: "", persona: "" });
  });
});
