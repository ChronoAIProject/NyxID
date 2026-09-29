import type { AssistantHttpMockHandler } from "@/lib/assistant/assistant-http";
import { mentionedNames } from "@/lib/assistant/nyxbot-mentions";
import {
  ASSISTANT_AGENT_DISPLAY_NAME_MAX,
  ASSISTANT_AGENT_NAME,
  ASSISTANT_AGENT_PERSONA_MAX,
  ASSISTANT_GROUP_MAX_MEMBERS,
  ASSISTANT_GROUP_NAME_MAX,
  type AssistantAgent,
  type AssistantAgentKind,
  type AssistantAgentMemoryNote,
  type AssistantGroup,
  type AssistantGroupMessage,
  type NyxAgentAcknowledgement,
  type NyxAgentChannelAgent,
  type NyxAgentConversation,
  type NyxAgentHistory,
  type NyxAgentMessageRole,
} from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
/** A 1x1 PNG a fixture camera tool "returns". */
const FIXTURE_PNG = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  ),
  (char) => char.charCodeAt(0),
);
const STORAGE = "nyxagent-http-fixture-v4";
export const NYXAGENT_FIXTURE_REPLY =
  "Your connected services are ready. [GitHub](/connect/nyx_clk_fixture_github)";
/** NyxBot delegates to the seeded researcher and is woken when it reports. */
export const NYXAGENT_FIXTURE_DELEGATE = "Ask the researcher for urgent issues";
export const NYXAGENT_FIXTURE_RESEARCHER_REPLY = "Found 3 urgent issues: #12, #15 and #18.";
const EVENT_HEADER =
  "NyxID events (authored by NyxID; only a quoted owner message is a request from the user):";
const SPECIALIST_WORK_MS = 2500;
/** How long the setup-link fixture waits for the "created" bot. */
const WAITING_MS = 3000;
export const NYXAGENT_FIXTURE_SETUP_BOT = "Set up a Telegram bot";
export const NYXAGENT_FIXTURE_BOT_LINKED =
  "Your Telegram bot @helper_bot is linked. Open https://t.me/helper_bot?start=nyxlink_fixture to verify your account.";
/** How long a group member "works" before its reply lands. */
const GROUP_WORK_MS = 1800;
/** Agent-to-agent hand-offs allowed per user message (the server bounds these too). */
const GROUP_HOPS = 6;
/** A seeded NyxBot thread that came from the user's Telegram bot. */
export const NYXAGENT_FIXTURE_CHANNEL_THREAD = "Morning briefing";
/** A user message that NyxBot hands to the researcher in a group. */
export const NYXAGENT_FIXTURE_GROUP_HANDOFF = "Find the urgent issues";
const LIMITS = {
  max_live_subagents_limit: 32,
  max_concurrent_subagent_turns_limit: 8,
  max_group_handoffs_limit: 24,
  max_group_handoffs_per_hour_limit: 600,
};
/** Channel bots the dev mock API lists (src/lib/mock-data.ts). */
const FIXTURE_BOTS: Readonly<Record<string, { platform: string; label: string; username: string }>> =
  {
    "bot-0001": { platform: "telegram", label: "NyxID Approvals", username: "nyxid_approvals_bot" },
    "bot-0002": { platform: "discord", label: "Dev Notifications", username: "NyxID Dev" },
  };

interface AgentRecord {
  id: string;
  kind: AssistantAgentKind;
  name: string;
  /** Optional in sessions persisted before display names existed. */
  display_name?: string | null;
  persona?: string | null;
  description: string;
  created_by: "user" | "nyxbot";
  services: string[];
  account_read: boolean;
  memory: AssistantAgentMemoryNote[];
  home_conversation_id: string | null;
  created_at: string;
  destroyed_at: string | null;
}

interface Row {
  history: NyxAgentHistory;
  agentId: string;
  settleAt?: number;
  notice?: boolean;
  reply?: string;
  /** NyxBot's thread to wake when this specialist turn settles. */
  reportTo?: string;
  /** When the thing this thread waits for happens (setup-link fixture). */
  waitingUntil?: number;
}

interface GroupReply {
  agentId: string;
  /** The text the member was addressed with. */
  text: string;
  at: number;
}

interface GroupRecord {
  id: string;
  name: string;
  member_agent_ids: string[];
  created_at: string;
  messages: AssistantGroupMessage[];
  /** Members still working, with when each reply lands. */
  queue: GroupReply[];
  hops: number;
}

interface Settings {
  skip_destructive_confirmation: boolean;
  max_live_subagents: number;
  max_concurrent_subagent_turns: number;
  max_group_handoffs: number;
  max_group_handoffs_per_hour: number;
}

interface State {
  rows: [string, Row][];
  agents: AgentRecord[];
  settings: Settings;
  channels: NyxAgentChannelAgent[];
  groups?: GroupRecord[];
}

const DEFAULT_SETTINGS: Settings = {
  skip_destructive_confirmation: false,
  max_live_subagents: 8,
  max_concurrent_subagent_turns: 3,
  max_group_handoffs: 6,
  max_group_handoffs_per_hour: 60,
};

const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json" },
  });
const failure = (status: number, message: string) => json({ message }, status);

const newConversationId = () => `nyxa-${crypto.randomUUID().replaceAll("-", "")}`;

function conversation(id: string, title: string, model: string, now: string): NyxAgentConversation {
  return {
    id,
    title,
    model,
    created_at: now,
    last_message_at: now,
    message_count: 0,
    pending_acknowledgements: 0,
    active_turn: null,
    context_reset_at: null,
    role: "orchestrator",
    agent: null,
    pending_events: 0,
    channel: null,
  };
}

function emptyHistory(id: string, title: string, model: string, now: string): NyxAgentHistory {
  return {
    conversation: conversation(id, title, model, now),
    messages: [],
    acknowledgements: [],
    approvals: [],
    waiting: [],
    before_seq: null,
  };
}

function acknowledgement(
  fields: Pick<NyxAgentAcknowledgement, "kind" | "summary"> & Partial<NyxAgentAcknowledgement>,
): NyxAgentAcknowledgement {
  return {
    id: crypto.randomUUID(),
    status: "pending",
    decider: "user",
    decided_by: null,
    reason: null,
    service_slug: null,
    service_name: null,
    tool_name: null,
    created_at: new Date().toISOString(),
    decided_at: null,
    expires_at: new Date(Date.now() + 900_000).toISOString(),
    ...fields,
  };
}

function note(text: string): AssistantAgentMemoryNote {
  const now = new Date().toISOString();
  return { id: crypto.randomUUID(), text, created_at: now, updated_at: now };
}

/**
 * Dev fixture only: simulates the server-owned NyxBot agents, threads,
 * memory, settings and channel bots across browser reload. NyxBot runs with
 * Full access: only destructive actions ask (unless the user turned that off).
 * Specialists use their grants and route other requests to NyxBot.
 */
export class NyxAgentHttpFixtures {
  private rows = new Map<string, Row>();
  private agents: AgentRecord[] = [];
  private settings: Settings = { ...DEFAULT_SETTINGS };
  private channels: NyxAgentChannelAgent[] = [];
  private groups: GroupRecord[] = [];

  constructor() {
    try {
      const state = JSON.parse(sessionStorage.getItem(STORAGE) ?? "null") as State | null;
      if (state) {
        this.rows = new Map(state.rows);
        this.agents = state.agents;
        this.settings = { ...DEFAULT_SETTINGS, ...state.settings };
        this.channels = state.channels;
        this.groups = state.groups ?? [];
      }
    } catch {
      this.rows.clear();
      this.agents = [];
    }
    if (!this.agents.length) this.seed();
  }

  /** NyxBot plus one specialist NyxBot created earlier, with a home thread and memory. */
  private seed() {
    const now = new Date().toISOString();
    this.agents = [
      {
        id: crypto.randomUUID(),
        kind: "nyxbot",
        name: "NyxBot",
        description: "",
        created_by: "user",
        services: [],
        account_read: true,
        memory: [],
        home_conversation_id: null,
        created_at: now,
        destroyed_at: null,
      },
    ];
    const researcher: AgentRecord = {
      id: crypto.randomUUID(),
      kind: "specialist",
      name: "researcher",
      description: "Tracks open GitHub issues and reports the most urgent ones.",
      created_by: "nyxbot",
      services: ["github"],
      account_read: false,
      memory: [
        note("The user wants weekly issue digests on Mondays."),
        note("Label urgent issues with the p0 tag."),
      ],
      home_conversation_id: null,
      created_at: now,
      destroyed_at: null,
    };
    this.agents.push(researcher);
    const home = this.thread(researcher, "researcher");
    const turnId = crypto.randomUUID();
    this.push(home, "orchestrator", "Find the three most urgent open GitHub issues.", turnId);
    this.push(home, "assistant", NYXAGENT_FIXTURE_RESEARCHER_REPLY, turnId);
    // A NyxBot thread that answers the user's Telegram bot.
    const briefing = this.thread(this.nyxbot(), NYXAGENT_FIXTURE_CHANNEL_THREAD);
    briefing.history.conversation.channel = { platform: "telegram" };
    const briefingTurn = crypto.randomUUID();
    this.push(briefing, "user", "What is on my calendar today?", briefingTurn);
    this.push(
      briefing,
      "assistant",
      "Two meetings: design review at 10:00 and a 1:1 at 15:00.",
      briefingTurn,
    );
    this.save();
  }

  private save() {
    const state: State = {
      rows: [...this.rows],
      agents: this.agents,
      settings: this.settings,
      channels: this.channels,
      groups: this.groups,
    };
    sessionStorage.setItem(STORAGE, JSON.stringify(state));
  }

  private nyxbot(): AgentRecord {
    return this.agents.find((agent) => agent.kind === "nyxbot")!;
  }

  private agentOf(row: Row): AgentRecord {
    return this.agents.find((agent) => agent.id === row.agentId) ?? this.nyxbot();
  }

  private threadsOf(agentId: string): Row[] {
    return [...this.rows.values()].filter((row) => row.agentId === agentId);
  }

  /** A new thread with an agent; its first thread becomes its home. */
  private thread(agent: AgentRecord, title: string, model = "nyxagent/chat"): Row {
    const id = newConversationId();
    const row: Row = {
      history: emptyHistory(id, title, model, new Date().toISOString()),
      agentId: agent.id,
    };
    this.rows.set(id, row);
    agent.home_conversation_id ??= id;
    return row;
  }

  private pending(row: Row): number {
    return row.history.acknowledgements.filter((ack) => ack.status === "pending").length;
  }

  /** The conversation DTO: the owning agent and pending cards are computed like the server does. */
  private view(row: Row): NyxAgentConversation {
    const agent = this.agentOf(row);
    return {
      ...row.history.conversation,
      role: agent.kind === "specialist" ? "subagent" : "orchestrator",
      agent: {
        id: agent.id,
        kind: agent.kind,
        name: agent.name,
        destroyed: Boolean(agent.destroyed_at),
      },
      pending_acknowledgements: this.pending(row),
    };
  }

  private agentView(agent: AgentRecord): AssistantAgent {
    const threads = this.threadsOf(agent.id);
    const replies = threads
      .flatMap((row) => row.history.messages)
      .filter((message) => message.role === "assistant")
      .sort((a, b) => a.created_at.localeCompare(b.created_at));
    const reply = replies.at(-1);
    const pending_requests = threads.flatMap((row) =>
      row.history.acknowledgements
        .filter((ack) => ack.status === "pending" && ack.decider === "orchestrator")
        .map((ack) => ({
          request_id: ack.id,
          agent: agent.name,
          agent_id: agent.id,
          conversation_id: row.history.conversation.id,
          kind: ack.kind,
          service_slug: ack.service_slug,
          summary: ack.summary,
          requested_by: null,
          expires_at: ack.expires_at,
        })),
    );
    const nyxbotId = this.nyxbot().id;
    return {
      id: agent.id,
      kind: agent.kind,
      name: agent.name,
      display_name: agent.display_name ?? null,
      persona: agent.persona ?? null,
      description: agent.description,
      specialty: null,
      created_by: agent.created_by,
      status: agent.destroyed_at
        ? "destroyed"
        : threads.some((row) => row.history.conversation.active_turn) ||
            // A group member speaks through a hidden thread of its own.
            this.groups.some((group) => group.queue.some((reply) => reply.agentId === agent.id))
          ? "running"
          : "idle",
      services: agent.services,
      account_read: agent.account_read,
      pending_requests,
      last_reply: reply
        ? { seq: reply.seq, status: reply.status, text: reply.text, created_at: reply.created_at }
        : null,
      home_conversation_id: agent.home_conversation_id,
      memory_count: agent.memory.length,
      created_at: agent.created_at,
      last_active_at: threads.reduce(
        (latest, row) =>
          row.history.conversation.last_message_at > latest
            ? row.history.conversation.last_message_at
            : latest,
        agent.created_at,
      ),
      destroyed_at: agent.destroyed_at,
      pending_acknowledgements: pending_requests.length,
      channels: this.channels
        .filter((channel) => (channel.agent_id ?? nyxbotId) === agent.id)
        .map((channel) => ({
          id: channel.id,
          platform: channel.platform,
          bot_label: channel.bot_label,
          status: channel.status,
        })),
    };
  }

  private push(row: Row, role: NyxAgentMessageRole, text: string, turnId: string) {
    const now = new Date().toISOString();
    row.history.messages.push({
      id: crypto.randomUUID(),
      seq: row.history.messages.length + 1,
      turn_id: turnId,
      role,
      text,
      status: "completed",
      error_code: null,
      created_at: now,
      activities: [],
      attachments: [],
    });
    row.history.conversation.message_count = row.history.messages.length;
    row.history.conversation.last_message_at = now;
  }

  /** A turn NyxID starts itself (an event or an instruction from NyxBot). */
  private startServerTurn(
    row: Row,
    role: "event" | "orchestrator",
    text: string,
    reply: string,
    delayMs = 1300,
  ) {
    const turnId = crypto.randomUUID();
    this.push(row, role, text, turnId);
    row.history.conversation.active_turn = {
      turn_id: turnId,
      started_at: new Date().toISOString(),
      activities: [],
      attachments: [],
    };
    row.reply = reply;
    row.settleAt = Date.now() + delayMs;
  }

  private settle(row: Row, cancelled = false) {
    const turn = row.history.conversation.active_turn;
    if (!turn) return;
    const now = new Date().toISOString();
    row.history.messages.push({
      id: crypto.randomUUID(),
      seq: row.history.messages.length + 1,
      turn_id: turn.turn_id,
      role: "assistant",
      text: cancelled ? "Your connected services" : (row.reply ?? NYXAGENT_FIXTURE_REPLY),
      status: cancelled ? "failed" : "completed",
      error_code: cancelled ? "cancelled" : null,
      created_at: now,
      activities: [],
      attachments: cancelled ? [] : turn.attachments,
    });
    row.history.conversation.active_turn = null;
    row.history.conversation.message_count = row.history.messages.length;
    row.history.conversation.last_message_at = now;
    if (cancelled) row.history.conversation.context_reset_at = now;
    delete row.settleAt;
    const reportTo = row.reportTo ? this.rows.get(row.reportTo) : undefined;
    delete row.reportTo;
    if (reportTo && !cancelled) {
      // NyxID wakes NyxBot's thread with an event turn (queued behind a live turn).
      const name = this.agentOf(row).name;
      const event =
        `${EVENT_HEADER}\n- Specialist ${name} replied. Reply excerpt: ` +
        `"${(row.reply ?? "").replaceAll('"', "'")}" Read more with nyxid__read_subagent.`;
      const answer = `The ${name} reported back: ${row.reply ?? ""}`;
      if (reportTo.history.conversation.active_turn) {
        const turnId = crypto.randomUUID();
        this.push(reportTo, "event", event, turnId);
        this.push(reportTo, "assistant", answer, turnId);
      } else {
        this.startServerTurn(reportTo, "event", event, answer);
      }
    }
    this.save();
  }

  private prepareSpecialistReply(row: Row, text: string) {
    const agent = this.agentOf(row);
    const service = /^Use (\w+)$/.exec(text)?.[1];
    if (service) {
      const slug = service.toLowerCase();
      if (agent.services.includes(slug)) {
        row.reply = `${service} lookup succeeded.`;
        return;
      }
      row.history.acknowledgements.push(
        acknowledgement({
          kind: "service",
          summary: `Use ${service}`,
          decider: "orchestrator",
          service_slug: slug,
          service_name: service,
        }),
      );
      row.reply = `I asked NyxBot for ${service} access. I will continue once it decides.`;
      return;
    }
    row.reply = `Working on it: ${text}`;
  }

  private prepareReply(row: Row, text: string) {
    if (this.agentOf(row).kind === "specialist") {
      this.prepareSpecialistReply(row, text);
      return;
    }
    const acknowledgements = row.history.acknowledgements;
    if (text === "Show the lobby camera") {
      // The camera tool returns an image during the turn; the server attaches it.
      row.history.conversation.active_turn!.attachments = [
        {
          id: crypto.randomUUID(),
          content_type: "image/png",
          size: FIXTURE_PNG.byteLength,
          label: "lobby-camera__snapshot",
        },
      ];
      row.reply = "Here is the latest lobby snapshot.";
      return;
    }
    if (text === NYXAGENT_FIXTURE_DELEGATE) {
      const researcher = this.agents.find(
        (agent) => agent.name === "researcher" && !agent.destroyed_at,
      );
      const home = researcher?.home_conversation_id
        ? this.rows.get(researcher.home_conversation_id)
        : undefined;
      if (!home || home.history.conversation.active_turn) {
        row.reply = "The researcher is not available right now.";
        return;
      }
      this.startServerTurn(
        home,
        "orchestrator",
        "Find the urgent open GitHub issues again.",
        NYXAGENT_FIXTURE_RESEARCHER_REPLY,
        SPECIALIST_WORK_MS,
      );
      home.reportTo = row.history.conversation.id;
      row.reply = "I asked the researcher to find the urgent issues.";
      return;
    }
    if (text === NYXAGENT_FIXTURE_SETUP_BOT) {
      // NyxID watches the setup link and resumes this thread when the bot
      // exists; until then the thread shows what it is waiting for.
      const now = new Date();
      row.history.waiting = [
        {
          kind: "channel_bot",
          title: "Waiting for your Telegram bot to be created",
          detail: null,
          since: now.toISOString(),
          expires_at: new Date(now.getTime() + 7_200_000).toISOString(),
        },
      ];
      row.waitingUntil = Date.now() + WAITING_MS;
      row.reply =
        "Open https://nyx.example/channel-bots/connect/telegram?label=Helper to create your bot. I will continue here when it exists.";
      return;
    }
    if (text.startsWith("Remember ")) {
      this.nyxbot().memory.push(note(text.slice("Remember ".length)));
      row.reply = "Noted. I will remember that.";
      return;
    }
    // Full access: services and account tools run without cards.
    if (text === "Use GitHub") {
      row.reply = "Repository lookup succeeded.";
      return;
    }
    if (text === "Manage my account") {
      row.reply = "Your agent keys are ready to manage.";
      return;
    }
    const previous = acknowledgements.at(-1);
    const confirmed = /^Confirmed: .*\(acknowledgement_id ([0-9a-f-]{36})\)\. Retry it now\.$/m.exec(
      text,
    );
    if (text === "Delete agent key ci-bot") {
      if (this.settings.skip_destructive_confirmation) {
        row.reply = "Deleted agent key ci-bot.";
        return;
      }
      acknowledgements.push(
        acknowledgement({
          kind: "action",
          summary: "Delete agent key 'ci-bot' (nyxid_ag_12345678)",
          tool_name: "nyxid__delete_agent_key",
        }),
      );
      row.reply = "Please confirm the deletion card, then I will continue.";
    } else if (confirmed) {
      const card = acknowledgements.find((ack) => ack.id === confirmed[1]);
      if (card?.status === "allowed") {
        card.status = "used";
        row.reply = "Deleted agent key ci-bot.";
      } else {
        row.reply = "That confirmation is no longer valid.";
      }
    } else if (previous?.status === "denied") {
      row.reply = "You denied this request. I will not proceed.";
    } else {
      row.reply = NYXAGENT_FIXTURE_REPLY;
    }
  }

  private settingsResponse() {
    return { ...this.settings, ...LIMITS };
  }

  private agentsRoute(url: URL, method: string, body: () => Record<string, unknown>) {
    const match = /^\/assistant\/nyxagent\/agents(?:\/([\w-]+))?(?:\/(grants|destroy|memory)(?:\/([\w-]+))?)?$/.exec(
      url.pathname,
    );
    if (!match) return undefined;
    const [, id, action, noteId] = match;
    if (!id) {
      if (method === "POST") return this.createAgent(body());
      const includeDestroyed = url.searchParams.get("include_destroyed") === "true";
      return json({
        agents: this.agents
          .filter((agent) => includeDestroyed || !agent.destroyed_at)
          .map((agent) => this.agentView(agent)),
        limits: this.settingsResponse(),
      });
    }
    const agent = this.agents.find((row) => row.id === id);
    if (!agent) return failure(404, "Agent not found");
    if (action === "grants" && method === "PUT") {
      if (agent.kind === "nyxbot") return failure(400, "NyxBot has full access");
      if (agent.destroyed_at) return failure(409, "That agent was destroyed");
      const update = body() as { services: string[]; account_read: boolean };
      agent.services = [...new Set(update.services)];
      agent.account_read = update.account_read;
      this.save();
      return json({ id: agent.id, services: agent.services, account_read: agent.account_read });
    }
    if (action === "destroy" && method === "POST") {
      if (agent.kind === "nyxbot") {
        return failure(400, "NyxBot cannot be destroyed; delete individual threads instead");
      }
      agent.destroyed_at ??= new Date().toISOString();
      this.channels = this.channels.filter((channel) => channel.agent_id !== agent.id);
      for (const group of this.groups) {
        group.queue = group.queue.filter((reply) => reply.agentId !== agent.id);
      }
      for (const row of this.threadsOf(agent.id)) {
        row.history.conversation.active_turn = null;
        delete row.settleAt;
        delete row.reportTo;
        for (const ack of row.history.acknowledgements) {
          if (ack.status === "pending") ack.status = "expired";
        }
      }
      this.save();
      return json({ id: agent.id, destroyed_at: agent.destroyed_at });
    }
    if (action === "memory" && noteId && method === "DELETE") {
      const before = agent.memory.length;
      agent.memory = agent.memory.filter((row) => row.id !== noteId);
      if (agent.memory.length === before) return failure(404, "Memory note not found");
      this.save();
      return new Response(null, { status: 204 });
    }
    if (action) return undefined;
    if (method === "GET") {
      return json({
        agent: this.agentView(agent),
        memory: agent.memory,
        threads: this.threadsOf(agent.id)
          .sort((a, b) =>
            b.history.conversation.last_message_at.localeCompare(
              a.history.conversation.last_message_at,
            ),
          )
          .map((row) => ({
            id: row.history.conversation.id,
            title: row.history.conversation.title,
            last_message_at: row.history.conversation.last_message_at,
            channel: row.history.conversation.channel,
            running: Boolean(row.history.conversation.active_turn),
          })),
      });
    }
    if (method === "PATCH") {
      const update = body() as {
        name?: string;
        description?: string;
        display_name?: string;
        persona?: string;
      };
      const style = this.styleRefusal(update);
      if (style) return style;
      if (update.name !== undefined && update.name !== agent.name) {
        if (agent.kind === "nyxbot") return failure(400, "NyxBot's name cannot be changed");
        const refusal = this.nameRefusal(update.name);
        if (refusal) return refusal;
        agent.name = update.name;
      }
      if (update.description !== undefined) agent.description = update.description.trim();
      // An empty string clears these.
      if (update.display_name !== undefined) agent.display_name = update.display_name.trim() || null;
      if (update.persona !== undefined) agent.persona = update.persona.trim() || null;
      this.save();
      return json({
        id: agent.id,
        name: agent.name,
        description: agent.description,
        display_name: agent.display_name ?? null,
        persona: agent.persona ?? null,
      });
    }
    if (method === "DELETE") {
      if (!agent.destroyed_at) return failure(409, "Destroy the agent before deleting it");
      for (const row of this.threadsOf(agent.id)) this.rows.delete(row.history.conversation.id);
      this.agents = this.agents.filter((row) => row.id !== agent.id);
      for (const group of this.groups) {
        group.member_agent_ids = group.member_agent_ids.filter((id) => id !== agent.id);
        group.queue = group.queue.filter((reply) => reply.agentId !== agent.id);
      }
      this.save();
      return new Response(null, { status: 204 });
    }
    return undefined;
  }

  /** Display name and persona rules, like the server's `style_value`. */
  private styleRefusal(fields: { display_name?: unknown; persona?: unknown }) {
    const checks = [
      [fields.display_name, ASSISTANT_AGENT_DISPLAY_NAME_MAX, "A display name", false],
      [fields.persona, ASSISTANT_AGENT_PERSONA_MAX, "A persona", true],
    ] as const;
    for (const [value, max, what, multiline] of checks) {
      if (value === undefined || value === null) continue;
      const text = String(value).trim();
      // eslint-disable-next-line no-control-regex
      const control = multiline ? /[\u0000-\u0009\u000b-\u001f\u007f]/ : /[\u0000-\u001f\u007f]/;
      if ([...text].length > max || control.test(text)) {
        return failure(400, `${what} must contain at most ${String(max)} characters`);
      }
    }
    return undefined;
  }

  private nameRefusal(name: string) {
    if (!ASSISTANT_AGENT_NAME.test(name)) return failure(400, "Invalid agent name");
    if (this.agents.some((agent) => agent.name === name && !agent.destroyed_at)) {
      return failure(409, "A live agent already uses that name");
    }
    return undefined;
  }

  private createAgent(body: Record<string, unknown>) {
    const name = String(body.name ?? "");
    const description = String(body.description ?? "").trim();
    const refusal = this.nameRefusal(name) ?? this.styleRefusal(body);
    if (refusal) return refusal;
    if (!description || description.length > 2048) {
      return failure(400, "Describe what this agent does");
    }
    const live = this.agents.filter((agent) => agent.kind === "specialist" && !agent.destroyed_at);
    if (live.length >= this.settings.max_live_subagents) {
      return failure(
        409,
        `You already have ${String(this.settings.max_live_subagents)} live agents; destroy one or raise the limit in settings`,
      );
    }
    const agent: AgentRecord = {
      id: crypto.randomUUID(),
      kind: "specialist",
      name,
      display_name: String(body.display_name ?? "").trim() || null,
      persona: String(body.persona ?? "").trim() || null,
      description,
      created_by: "user",
      services: [...new Set((body.services as string[] | undefined) ?? [])],
      account_read: body.account_read === true,
      memory: [],
      home_conversation_id: null,
      created_at: new Date().toISOString(),
      destroyed_at: null,
    };
    this.agents.push(agent);
    const home = this.thread(agent, name);
    this.save();
    return json({ id: agent.id, name, home_conversation_id: home.history.conversation.id }, 201);
  }

  private channelsRoute(url: URL, method: string, body: () => Record<string, unknown>) {
    if (url.pathname === `${ROOT}/channels`) {
      if (method === "POST") {
        const request = body() as { bot_id: string; agent_id?: string };
        return this.connectChannel(request.bot_id, request.agent_id);
      }
      return json({ channel_agents: this.channels });
    }
    const match = /\/channels\/([\w-]+)$/.exec(url.pathname);
    if (!match) return undefined;
    const row = this.channels.find((channel) => channel.id === match[1]);
    if (!row) return failure(404, "Channel agent not found");
    if (method === "DELETE") {
      this.channels = this.channels.filter((channel) => channel.id !== row.id);
      this.save();
      return json({ disconnected: row.id, platform: row.platform, gateway_released: true });
    }
    if (method === "PATCH") {
      const agent = this.agents.find((candidate) => candidate.id === body().agent_id);
      if (!agent) return failure(404, "Agent not found");
      if (agent.destroyed_at) return failure(409, "That agent was destroyed");
      const changed = (row.agent_id ?? this.nyxbot().id) !== agent.id;
      row.agent_id = agent.id;
      this.save();
      return json({ channel_agent_id: row.id, agent: agent.name, changed });
    }
    return undefined;
  }

  private connectChannel(botId: string, agentId: string | undefined) {
    const bot = FIXTURE_BOTS[botId];
    if (!bot) return failure(404, "Channel bot not found");
    const agent = agentId ? this.agents.find((candidate) => candidate.id === agentId) : undefined;
    if (agentId && !agent) return failure(404, "Agent not found");
    if (agent?.destroyed_at) return failure(409, "That agent was destroyed");
    let row = this.channels.find((channel) => channel.channel_bot_id === botId);
    if (!row) {
      row = {
        id: crypto.randomUUID(),
        channel_bot_id: botId,
        platform: bot.platform,
        bot_label: bot.label,
        bot_username: bot.username,
        transport: bot.platform === "telegram" ? "gateway" : "direct",
        status: "active",
        last_error: null,
        owner_linked: false,
        agent_id: null,
        org_id: null,
        delivery_status: null,
        delivery_error: null,
        delivery_reason: null,
        delivery_failed_at: null,
        inbound_hint: null,
        created_at: new Date().toISOString(),
      };
      this.channels.unshift(row);
    }
    // Re-posting refreshes the link and relinks to the requested agent.
    row.agent_id = agent?.id ?? null;
    this.save();
    const code = `nyxlink_${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
    const url = bot.platform === "telegram" ? `https://t.me/${bot.username}?start=${code}` : null;
    return json({
      channel_agent: row,
      link: {
        code,
        url,
        expires_at: new Date(Date.now() + 86_400_000).toISOString(),
        instructions: url
          ? "Open the link on the phone or computer where you use Telegram and press Start. " +
            "That links your Telegram account as NyxBot's owner."
          : `Send this code to the bot once from your own ${bot.platform} account: ${code}`,
      },
    });
  }

  // ---- Groups -------------------------------------------------------------

  private memberName(agent: AgentRecord): string {
    return agent.kind === "nyxbot" ? "NyxBot" : agent.name;
  }

  private membersOf(group: GroupRecord): AgentRecord[] {
    return group.member_agent_ids
      .map((id) => this.agents.find((agent) => agent.id === id))
      .filter((agent): agent is AgentRecord => Boolean(agent));
  }

  /** NyxBot when it is a member, else the first member. */
  private leadOf(group: GroupRecord): string {
    const members = this.membersOf(group);
    return (members.find((agent) => agent.kind === "nyxbot") ?? members[0])?.id ?? "";
  }

  private groupView(group: GroupRecord): AssistantGroup {
    const members = this.membersOf(group);
    const working = members
      .filter((agent) => group.queue.some((reply) => reply.agentId === agent.id))
      .map((agent) => agent.id);
    return {
      id: group.id,
      name: group.name,
      members: members.map((agent) => ({
        id: agent.id,
        name: this.memberName(agent),
        kind: agent.kind,
        destroyed: Boolean(agent.destroyed_at),
        working: working.includes(agent.id),
      })),
      lead_agent_id: this.leadOf(group),
      working_agent_ids: working,
      message_count: group.messages.length,
      last_message_at: group.messages.at(-1)?.created_at ?? null,
      created_at: group.created_at,
    };
  }

  private postGroup(
    group: GroupRecord,
    role: AssistantGroupMessage["role"],
    text: string,
    agent?: AgentRecord,
  ): AssistantGroupMessage {
    const message: AssistantGroupMessage = {
      id: crypto.randomUUID(),
      seq: (group.messages.at(-1)?.seq ?? 0) + 1,
      role,
      agent: agent ? { id: agent.id, name: this.memberName(agent), kind: agent.kind } : null,
      text,
      created_at: new Date().toISOString(),
    };
    group.messages.push(message);
    return message;
  }

  /** Queue a member's reply after whatever it is already working on. */
  private addressGroup(group: GroupRecord, agentId: string, text: string) {
    const busyUntil = group.queue
      .filter((reply) => reply.agentId === agentId)
      .reduce((latest, reply) => Math.max(latest, reply.at), Date.now());
    group.queue.push({ agentId, text, at: busyUntil + GROUP_WORK_MS });
  }

  private groupReply(group: GroupRecord, agent: AgentRecord, text: string): string {
    const names = this.membersOf(group).map((member) => this.memberName(member));
    const clean = text.replace(/(^|\s)@[A-Za-z0-9-]+/g, " ").replace(/\s+/g, " ").trim();
    const researcher = this.membersOf(group).find(
      (member) => member.name === "researcher" && !member.destroyed_at,
    );
    if (agent.kind === "nyxbot" && /urgent issues/i.test(clean) && researcher && names.length > 1) {
      // NyxBot hands the work to the specialist by mentioning it.
      return "@researcher can you find the urgent issues?";
    }
    if (agent.name === "researcher" && /urgent issues/i.test(clean)) {
      return NYXAGENT_FIXTURE_RESEARCHER_REPLY;
    }
    return agent.kind === "nyxbot" ? `Noted: ${clean}` : `Working on it: ${clean}`;
  }

  /** Land every reply that is due; replies that mention members hand work on. */
  private settleGroups() {
    let changed = false;
    for (const group of this.groups) {
      for (;;) {
        group.queue.sort((a, b) => a.at - b.at);
        const due = group.queue[0];
        if (!due || due.at > Date.now()) break;
        group.queue.shift();
        const agent = this.agents.find((row) => row.id === due.agentId);
        if (!agent || agent.destroyed_at) continue;
        const reply = this.groupReply(group, agent, due.text);
        this.postGroup(group, "agent", reply, agent);
        changed = true;
        const members = this.membersOf(group);
        const handoffs = mentionedNames(
          reply,
          members.map((member) => this.memberName(member)),
        )
          .map((name) => members.find((member) => this.memberName(member) === name)!)
          .filter((member) => member.id !== agent.id && !member.destroyed_at);
        for (const member of handoffs) {
          if (group.hops <= 0) break;
          group.hops -= 1;
          this.addressGroup(group, member.id, reply);
        }
      }
    }
    if (changed) this.save();
  }

  /**
   * Validation like the server's (services/assistant_group_service.rs):
   * duplicate ids collapse, unknown agents are 400 and destroyed agents 409,
   * even when they are already members.
   */
  private groupRefusal(name: unknown, memberIds: unknown) {
    if (name !== undefined) {
      const trimmed = typeof name === "string" ? name.trim() : "";
      if (!trimmed || [...trimmed].length > ASSISTANT_GROUP_NAME_MAX) {
        return failure(400, `A group name has 1 to ${String(ASSISTANT_GROUP_NAME_MAX)} characters`);
      }
    }
    if (memberIds !== undefined) {
      const ids = [...new Set(Array.isArray(memberIds) ? (memberIds as unknown[]) : [])];
      for (const id of ids) {
        const agent = this.agents.find((row) => row.id === id);
        if (!agent) return failure(400, "Unknown agent in member_agent_ids");
        if (agent.destroyed_at) return failure(409, `Agent ${agent.name} was destroyed`);
      }
      if (!ids.length || ids.length > ASSISTANT_GROUP_MAX_MEMBERS) {
        return failure(400, `A group has 1 to ${String(ASSISTANT_GROUP_MAX_MEMBERS)} agents`);
      }
    }
    return undefined;
  }

  private groupsRoute(url: URL, method: string, body: () => Record<string, unknown>) {
    const match = /^\/assistant\/nyxagent\/groups(?:\/([\w-]+))?(\/messages)?$/.exec(url.pathname);
    if (!match) return undefined;
    const [, id, messages] = match;
    if (!id) {
      if (method === "POST") {
        const request = body();
        const refusal = this.groupRefusal(request.name ?? "", request.member_agent_ids ?? []);
        if (refusal) return refusal;
        const group: GroupRecord = {
          id: `nyxg-${crypto.randomUUID().replaceAll("-", "")}`,
          name: String(request.name).trim(),
          member_agent_ids: [...new Set(request.member_agent_ids as string[])],
          created_at: new Date().toISOString(),
          messages: [],
          queue: [],
          hops: GROUP_HOPS,
        };
        this.groups.push(group);
        const names = this.membersOf(group).map((agent) => this.memberName(agent));
        this.postGroup(group, "notice", `Group created with ${names.join(", ")}.`);
        this.save();
        return json(this.groupView(group), 201);
      }
      return json({
        groups: this.groups
          .map((group) => this.groupView(group))
          .sort((a, b) =>
            (b.last_message_at ?? b.created_at).localeCompare(a.last_message_at ?? a.created_at),
          ),
      });
    }
    const group = this.groups.find((row) => row.id === id);
    if (!group) return failure(404, "Group not found");
    if (messages) {
      if (method === "POST") {
        const text = String(body().text ?? "").trim();
        if (!text || [...text].length > 32768) {
          return failure(400, "A message has 1 to 32768 characters");
        }
        // Like the server: destroyed members cannot be addressed, so a message
        // that only mentions them goes to the lead (while it is alive).
        const live = this.membersOf(group).filter((agent) => !agent.destroyed_at);
        const mentioned = mentionedNames(
          text,
          live.map((agent) => this.memberName(agent)),
        ).map((name) => live.find((agent) => this.memberName(agent) === name)!);
        const lead = live.find((agent) => agent.id === this.leadOf(group));
        const addressed = mentioned.length ? mentioned : lead ? [lead] : [];
        const message = this.postGroup(group, "user", text);
        group.hops = GROUP_HOPS;
        for (const agent of addressed) this.addressGroup(group, agent.id, text);
        this.save();
        return json({ message, addressed_agent_ids: addressed.map((agent) => agent.id) }, 202);
      }
      const limit = Math.min(Number(url.searchParams.get("limit")) || 50, 100);
      const before = Number(url.searchParams.get("before_seq")) || Number.POSITIVE_INFINITY;
      const older = group.messages.filter((message) => message.seq < before);
      const page = older.slice(-limit);
      return json({
        group: this.groupView(group),
        messages: page,
        before_seq: older.length > page.length ? (page[0]?.seq ?? null) : null,
      });
    }
    if (method === "GET") return json(this.groupView(group));
    if (method === "DELETE") {
      if (group.queue.length) {
        return json(
          { message: "A turn is already active in this conversation", error: "turn_active" },
          409,
        );
      }
      this.groups = this.groups.filter((row) => row.id !== group.id);
      this.save();
      return new Response(null, { status: 204 });
    }
    if (method === "PATCH") {
      const update = body() as { name?: string; member_agent_ids?: string[] };
      const refusal = this.groupRefusal(update.name, update.member_agent_ids);
      if (refusal) return refusal;
      if (update.name !== undefined) group.name = update.name.trim();
      if (update.member_agent_ids) {
        const before = this.membersOf(group);
        const after = [...new Set(update.member_agent_ids)];
        group.member_agent_ids = after;
        group.queue = group.queue.filter((reply) => after.includes(reply.agentId));
        const names = (agents: AgentRecord[]) =>
          agents.map((agent) => this.memberName(agent)).join(", ");
        const joined = this.membersOf(group).filter(
          (agent) => !before.some((row) => row.id === agent.id),
        );
        const left = before.filter((agent) => !after.includes(agent.id));
        const parts = [
          ...(joined.length ? [`${names(joined)} joined.`] : []),
          ...(left.length ? [`${names(left)} left.`] : []),
        ];
        if (parts.length) this.postGroup(group, "notice", parts.join(" "));
      }
      this.save();
      return json(this.groupView(group));
    }
    return undefined;
  }

  /** Open live streams (only when the `nyxagentLive` fault enables them). */
  private liveStreams = new Set<ReadableStreamDefaultController<Uint8Array>>();
  private liveTimer: ReturnType<typeof setInterval> | undefined;
  private liveSignatures = new Map<string, string>();

  /** What a live `conversation`/`group` event would report as changed. */
  private signatures() {
    const signatures = new Map<string, string>();
    for (const [id, row] of this.rows) {
      const conversation = row.history.conversation;
      signatures.set(
        `conversation:${id}`,
        [
          row.history.messages.length,
          conversation.active_turn?.turn_id ?? "",
          row.history.waiting.length,
          conversation.pending_events,
        ].join(":"),
      );
    }
    for (const group of this.groups) {
      signatures.set(`group:${group.id}`, `${String(group.messages.length)}:${String(group.queue.length)}`);
    }
    return signatures;
  }

  private emitLive() {
    if (!this.liveStreams.size) return;
    const encoder = new TextEncoder();
    const next = this.signatures();
    for (const [key, signature] of next) {
      if (this.liveSignatures.get(key) === signature) continue;
      const [kind, id] = key.split(/:(.*)/s) as [string, string];
      const row = kind === "conversation" ? this.rows.get(id) : undefined;
      const event = row
        ? {
            type: "conversation",
            id,
            group_id: null,
            turn_id: row.history.conversation.active_turn?.turn_id ?? null,
            messages: row.history.messages.length,
          }
        : { type: "group", id };
      const frame = encoder.encode(`data: ${JSON.stringify(event)}\n\n`);
      for (const controller of this.liveStreams) controller.enqueue(frame);
    }
    this.liveSignatures = next;
  }

  /** NyxID's server-side progress: turns settle, waits resolve, groups answer. */
  private tick() {
    for (const row of [...this.rows.values()]) {
      if (row.settleAt && row.settleAt <= Date.now()) this.settle(row);
      if (
        row.waitingUntil &&
        row.waitingUntil <= Date.now() &&
        !row.history.conversation.active_turn
      ) {
        row.history.waiting = [];
        delete row.waitingUntil;
        this.startServerTurn(
          row,
          "event",
          "The Telegram channel bot Helper the user just created is now linked to nyxbot.",
          NYXAGENT_FIXTURE_BOT_LINKED,
          800,
        );
      }
    }
    this.settleGroups();
    this.emitLive();
  }

  private live(): Response {
    const encoder = new TextEncoder();
    let own: ReadableStreamDefaultController<Uint8Array> | undefined;
    const stream = new ReadableStream<Uint8Array>({
      start: (controller) => {
        own = controller;
        this.liveStreams.add(controller);
        this.liveSignatures = this.signatures();
        controller.enqueue(encoder.encode(`data: ${JSON.stringify({ type: "ready" })}\n\n`));
        this.liveTimer ??= setInterval(() => this.tick(), 200);
      },
      cancel: () => {
        if (own) this.liveStreams.delete(own);
        if (!this.liveStreams.size && this.liveTimer) {
          clearInterval(this.liveTimer);
          this.liveTimer = undefined;
        }
      },
    });
    return new Response(stream, { status: 200, headers: { "content-type": "text/event-stream" } });
  }

  readonly handler: AssistantHttpMockHandler = async ({ endpoint, init }) => {
    if (!endpoint.startsWith(ROOT)) return undefined;
    this.tick();
    const url = new URL(endpoint, window.location.origin);
    const method = init.method ?? "GET";
    if (url.pathname === `${ROOT}/live`) {
      return globalThis.__nyxidAssistantHttpFaults?.nyxagentLive
        ? this.live()
        : failure(404, "Assistant route not found.");
    }
    const body = () => JSON.parse(String(init.body)) as Record<string, unknown>;
    const groups = this.groupsRoute(url, method, body);
    if (groups) return groups;
    const agents = this.agentsRoute(url, method, body);
    if (agents) return agents;
    const channels = this.channelsRoute(url, method, body);
    if (channels) return channels;
    if (url.pathname === `${ROOT}/conversations`) {
      const agentId = url.searchParams.get("agent_id");
      if (agentId && !this.agents.some((agent) => agent.id === agentId)) {
        return failure(404, "Agent not found");
      }
      return json({
        conversations: [...this.rows.values()]
          .filter((row) => !agentId || row.agentId === agentId)
          .map((row) => this.view(row)),
        next_cursor: null,
      });
    }
    if (url.pathname === `${ROOT}/settings`) {
      if (method === "PUT") {
        const update = body() as Partial<Settings>;
        const live = update.max_live_subagents;
        const turns = update.max_concurrent_subagent_turns;
        const handoffs = update.max_group_handoffs;
        const hourly = update.max_group_handoffs_per_hour;
        if (
          (live !== undefined && (live < 0 || live > LIMITS.max_live_subagents_limit)) ||
          (turns !== undefined &&
            (turns < 1 || turns > LIMITS.max_concurrent_subagent_turns_limit)) ||
          (handoffs !== undefined &&
            (handoffs < 0 || handoffs > LIMITS.max_group_handoffs_limit)) ||
          (hourly !== undefined && (hourly < 0 || hourly > LIMITS.max_group_handoffs_per_hour_limit))
        ) {
          return failure(400, "Setting out of range");
        }
        this.settings = { ...this.settings, ...update };
        this.save();
      }
      return json(this.settingsResponse());
    }
    const attachmentRoute = /\/conversations\/(nyxa-[a-f0-9]{32})\/attachments\/([^/]+)$/.exec(
      url.pathname,
    );
    if (attachmentRoute && method === "GET") {
      const row = this.rows.get(attachmentRoute[1]!);
      const known = [
        ...(row?.history.messages.flatMap((message) => message.attachments) ?? []),
        ...(row?.history.conversation.active_turn?.attachments ?? []),
      ].some((attachment) => attachment.id === attachmentRoute[2]);
      if (!known) return failure(404, "Attachment not found");
      return new Response(FIXTURE_PNG, { headers: { "content-type": "image/png" } });
    }
    if (/\/conversations\/(nyxa-[a-f0-9]{32})\/access-mode$/.test(url.pathname) && method === "PATCH") {
      return json(
        {
          error: "access_mode_retired",
          message: "Every NyxBot chat runs with Full access; the Ask/Full choice was removed.",
        },
        410,
      );
    }
    const decisionRoute = /\/conversations\/(nyxa-[a-f0-9]{32})\/acknowledgements\/([\w-]+)$/.exec(
      url.pathname,
    );
    if (decisionRoute && method === "POST") {
      const row = this.rows.get(decisionRoute[1]!);
      const acknowledgement = row?.history.acknowledgements.find(
        (ack) => ack.id === decisionRoute[2],
      );
      if (!row || !acknowledgement) return failure(404, "Acknowledgement not found");
      if (acknowledgement.status !== "pending") {
        return failure(409, "Acknowledgement is no longer pending");
      }
      const allow = body().decision === "allow";
      acknowledgement.status = allow ? "allowed" : "denied";
      acknowledgement.decided_at = new Date().toISOString();
      acknowledgement.decided_by = "user";
      if (acknowledgement.kind === "action" && allow) {
        acknowledgement.expires_at = new Date(Date.now() + 600_000).toISOString();
      }
      if (acknowledgement.decider === "orchestrator") {
        // NyxID resumes the specialist itself with an event turn.
        const agent = this.agentOf(row);
        const service = acknowledgement.service_name ?? "the service";
        if (allow && acknowledgement.service_slug) {
          agent.services = [...new Set([...agent.services, acknowledgement.service_slug])];
        }
        const target = acknowledgement.service_slug
          ? `service ${acknowledgement.service_slug}`
          : "read-only account access";
        this.startServerTurn(
          row,
          "event",
          `${EVENT_HEADER}\n- The user ${allow ? "allowed" : "denied"} your request for ${target}. ${
            allow
              ? "Retry the call now and continue your task."
              : "Do not retry it; finish what you can and report."
          }`,
          allow ? `${service} access granted. Lookup succeeded.` : `Understood. I will not use ${service}.`,
        );
      }
      this.save();
      return json(acknowledgement);
    }
    const match = /\/conversations\/(nyxa-[a-f0-9]{32})(\/stop)?$/.exec(url.pathname);
    if (match) {
      const row = this.rows.get(match[1]!);
      if (!row) return failure(404, "Conversation not found");
      if (match[2] && method === "POST") {
        this.settle(row, true);
        return new Response(null, { status: 204 });
      }
      if (method === "GET") return json({ ...row.history, conversation: this.view(row) });
      if (row.history.conversation.active_turn) {
        return json({ message: "A turn is already active", error: "turn_active" }, 409);
      }
      if (method === "DELETE") {
        this.rows.delete(match[1]!);
        const agent = this.agentOf(row);
        if (agent.home_conversation_id === match[1]) agent.home_conversation_id = null;
        this.save();
        return new Response(null, { status: 204 });
      }
      if (method === "PATCH") {
        row.history.conversation.title = String(body().title);
        this.save();
        // Like the server, a rename response carries neither the agent nor pending cards.
        return json({ ...row.history.conversation, agent: null, pending_acknowledgements: 0 });
      }
    }
    if (url.pathname === `${ROOT}/turns` && method === "POST") {
      const request = body() as {
        conversation_id?: string;
        agent_id?: string;
        text: string;
        model?: string;
      };
      if (request.conversation_id && request.agent_id) {
        return failure(400, "agent_id only applies to new threads");
      }
      let row = request.conversation_id ? this.rows.get(request.conversation_id) : undefined;
      if (request.conversation_id && !row) return failure(404, "Conversation not found");
      if (!row) {
        const agent = request.agent_id
          ? this.agents.find((candidate) => candidate.id === request.agent_id)
          : this.nyxbot();
        if (!agent) return failure(404, "Agent not found");
        if (agent.destroyed_at) return failure(409, "That agent was destroyed");
        row = this.thread(
          agent,
          [...request.text].slice(0, 40).join(""),
          request.model ?? "nyxagent/chat",
        );
      }
      if (this.agentOf(row).destroyed_at) {
        return failure(409, "This agent was destroyed; its threads are read-only");
      }
      if (row.history.conversation.active_turn) {
        return failure(409, "A turn is already active");
      }
      const id = row.history.conversation.id;
      const now = new Date().toISOString();
      const turn = crypto.randomUUID();
      const message = crypto.randomUUID();
      const block = `${message}-text`;
      const rebind = request.text.includes("reset context");
      row.notice = Boolean(row.history.conversation.context_reset_at) || rebind;
      if (rebind) row.history.conversation.context_reset_at = now;
      row.history.conversation.active_turn = {
        turn_id: turn,
        started_at: now,
        activities: [],
        attachments: [],
      };
      this.push(row, "user", request.text, turn);
      this.prepareReply(row, request.text);
      row.settleAt = Date.now() + (globalThis.__nyxidAssistantHttpFaults?.progressStallMs ?? 1300);
      this.save();
      const current = row;
      let cursor = 0;
      let timer: ReturnType<typeof setInterval>;
      const encoder = new TextEncoder();
      const stream = new ReadableStream<Uint8Array>({
        start: (controller) => {
          const emit = (event: string, data: object) => {
            const payload = JSON.stringify({ event, cursor: ++cursor, ...data });
            controller.enqueue(encoder.encode(`data: ${payload}\n\n`));
          };
          emit("turn.status", { conversation_id: id, turn_id: turn, status: "running" });
          emit("message.started", { message_id: message, role: "assistant" });
          emit("block.started", {
            message_id: message,
            block_id: block,
            index: 0,
            block: { type: "text", block_id: block, text: "" },
          });
          if (current.notice) {
            emit("turn.notice", { code: "context_reset", message: "Context reset" });
          }
          emit("block.delta", { block_id: block, text: "Your connected services" });
          timer = setInterval(() => {
            if (current.settleAt && current.settleAt <= Date.now()) this.settle(current);
            if (current.history.conversation.active_turn?.turn_id === turn) return;
            // Look the reply up by turn: a server event turn may already follow it.
            const reply = current.history.messages.find(
              (entry) => entry.turn_id === turn && entry.role === "assistant",
            );
            emit("block.completed", {
              block_id: block,
              block: { type: "text", block_id: block, text: reply?.text ?? "" },
            });
            emit("message.completed", { message_id: message });
            emit("turn.completed", {
              turn_id: turn,
              status: reply?.error_code === "cancelled" ? "cancelled" : "completed",
              error: reply?.error_code ? { code: "cancelled", message: "Stopped." } : null,
            });
            clearInterval(timer);
            controller.close();
          }, 50);
        },
        cancel: () => {
          clearInterval(timer);
          // Persisted deadline still settles on reload/poll.
        },
      });
      return new Response(stream, { headers: { "content-type": "text/event-stream" } });
    }
    return failure(404, "Fixture route not found");
  };
}
