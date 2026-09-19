import type { AssistantHttpMockHandler } from "@/lib/assistant/assistant-http";
import type { NyxAgentHistory } from "@/schemas/assistant-nyxagent";

const ROOT = "/assistant/nyxagent";
const STORAGE = "nyxagent-http-fixture";
export const NYXAGENT_FIXTURE_REPLY =
  "Your connected services are ready. [GitHub](/connect/nyx_clk_fixture_github)";

interface Record {
  history: NyxAgentHistory;
  settleAt?: number;
  notice?: boolean;
  reply?: string;
}

const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json" },
  });

/** Dev fixture only: simulates the server-owned transcript across browser reload. */
export class NyxAgentHttpFixtures {
  private rows = new Map<string, Record>();

  constructor() {
    try {
      this.rows = new Map(
        JSON.parse(sessionStorage.getItem(STORAGE) ?? "[]") as [string, Record][],
      );
    } catch {
      this.rows.clear();
    }
  }

  private save() {
    sessionStorage.setItem(STORAGE, JSON.stringify([...this.rows]));
  }

  private settle(row: Record, cancelled = false) {
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
    });
    row.history.conversation.active_turn = null;
    row.history.conversation.message_count = row.history.messages.length;
    row.history.conversation.last_message_at = now;
    if (cancelled) row.history.conversation.context_reset_at = now;
    delete row.settleAt;
    this.save();
  }

  private prepareReply(row: Record, text: string) {
    const acknowledgements = row.history.acknowledgements;
    if (row.history.conversation.access_mode === "full") {
      row.reply =
        text === "Delete agent key ci-bot"
          ? "Deleted agent key ci-bot."
          : "Full access: the requested operation completed.";
      return;
    }
    const kind =
      text === "Use GitHub"
        ? "service"
        : text === "Manage my account"
          ? "account"
          : text === "Delete agent key ci-bot"
            ? "action"
            : undefined;
    const previous = acknowledgements.at(-1);
    if (kind && !acknowledgements.some((ack) => ack.kind === kind && ack.status === "allowed")) {
      const now = new Date().toISOString();
      acknowledgements.push({
        id: crypto.randomUUID(),
        kind,
        status: "pending",
        summary:
          kind === "action"
            ? "Delete agent key 'ci-bot' (nyxid_ag_12345678)"
            : kind === "account"
              ? "Manage your NyxID account"
              : "Use GitHub in this chat",
        service_slug: kind === "service" ? "github" : null,
        service_name: kind === "service" ? "GitHub" : null,
        tool_name: kind === "action" ? "nyxid__delete_agent_key" : null,
        created_at: now,
        decided_at: null,
        expires_at: new Date(Date.now() + 900_000).toISOString(),
      });
      row.reply = "Please review the permission card, then tell me to continue.";
    } else if (previous?.status === "denied") {
      row.reply = "You denied this request. I will not proceed.";
    } else if (previous?.status === "allowed") {
      row.reply =
        previous.kind === "action"
          ? "Deleted agent key ci-bot."
          : previous.kind === "service"
            ? "GitHub access granted. Repository lookup succeeded."
            : "Account access granted. Your agent keys are ready to manage.";
      if (previous.kind === "action") previous.status = "used";
    } else {
      row.reply = NYXAGENT_FIXTURE_REPLY;
    }
    row.history.conversation.pending_acknowledgements = acknowledgements.filter(
      (ack) => ack.status === "pending",
    ).length;
  }

  readonly handler: AssistantHttpMockHandler = async ({ endpoint, init }) => {
    if (!endpoint.startsWith(ROOT)) return undefined;
    for (const row of this.rows.values()) {
      if (row.settleAt && row.settleAt <= Date.now()) this.settle(row);
    }
    const url = new URL(endpoint, window.location.origin);
    const method = init.method ?? "GET";
    if (url.pathname === `${ROOT}/models`) {
      return json([
        { id: "nyxagent/chat", label: "chat" },
        { id: "nyxagent/research", label: "research" },
      ]);
    }
    if (url.pathname === `${ROOT}/conversations`) {
      return json({
        conversations: [...this.rows.values()].map((row) => row.history.conversation),
        next_cursor: null,
      });
    }
    const modeRoute = /\/conversations\/(nyxa-[a-f0-9]{32})\/access-mode$/.exec(url.pathname);
    if (modeRoute && method === "PATCH") {
      const row = this.rows.get(modeRoute[1]!);
      if (!row) return json({ message: "Conversation not found" }, 404);
      if (row.history.conversation.active_turn) {
        return json({ message: "A turn is already active" }, 409);
      }
      const body = JSON.parse(String(init.body)) as { access_mode: "ask" | "full" };
      row.history.conversation.access_mode = body.access_mode;
      row.history.conversation.pending_acknowledgements = 0;
      for (const acknowledgement of row.history.acknowledgements) {
        if (acknowledgement.status === "pending") acknowledgement.status = "expired";
      }
      this.save();
      return json(row.history.conversation);
    }
    const decisionRoute = /\/conversations\/(nyxa-[a-f0-9]{32})\/acknowledgements\/([\w-]+)$/.exec(
      url.pathname,
    );
    if (decisionRoute && method === "POST") {
      const row = this.rows.get(decisionRoute[1]!);
      const acknowledgement = row?.history.acknowledgements.find(
        (ack) => ack.id === decisionRoute[2],
      );
      if (!row || !acknowledgement) return json({ message: "Acknowledgement not found" }, 404);
      if (acknowledgement.status !== "pending") {
        return json({ message: "Acknowledgement is no longer pending" }, 409);
      }
      const body = JSON.parse(String(init.body)) as { decision: "allow" | "deny" };
      acknowledgement.status = body.decision === "allow" ? "allowed" : "denied";
      acknowledgement.decided_at = new Date().toISOString();
      if (acknowledgement.kind === "action" && body.decision === "allow") {
        acknowledgement.expires_at = new Date(Date.now() + 600_000).toISOString();
      }
      row.history.conversation.pending_acknowledgements = row.history.acknowledgements.filter(
        (ack) => ack.status === "pending",
      ).length;
      this.save();
      return json(acknowledgement);
    }
    const match = /\/conversations\/(nyxa-[a-f0-9]{32})(\/stop)?$/.exec(url.pathname);
    if (match) {
      const row = this.rows.get(match[1]!);
      if (!row) return json({ message: "Conversation not found" }, 404);
      if (match[2] && method === "POST") {
        this.settle(row, true);
        return new Response(null, { status: 204 });
      }
      if (method === "GET") return json(row.history);
      if (row.history.conversation.active_turn) {
        return json({ message: "A turn is already active", error: "turn_active" }, 409);
      }
      if (method === "DELETE") {
        this.rows.delete(match[1]!);
        this.save();
        return new Response(null, { status: 204 });
      }
      if (method === "PATCH") {
        const body = JSON.parse(String(init.body)) as { title: string };
        row.history.conversation.title = body.title;
        this.save();
        return json(row.history.conversation);
      }
    }
    if (url.pathname === `${ROOT}/turns` && method === "POST") {
      const body = JSON.parse(String(init.body)) as {
        conversation_id?: string;
        text: string;
        model?: string;
        access_mode?: "ask" | "full";
      };
      const id = body.conversation_id ?? `nyxa-${crypto.randomUUID().replaceAll("-", "")}`;
      const now = new Date().toISOString();
      let row = this.rows.get(id);
      if (body.conversation_id && !row) return json({ message: "Conversation not found" }, 404);
      if (!row) {
        row = {
          history: {
            conversation: {
              id,
              title: [...body.text].slice(0, 40).join(""),
              model: body.model ?? "nyxagent/chat",
              access_mode: body.access_mode ?? "ask",
              created_at: now,
              last_message_at: now,
              message_count: 0,
              pending_acknowledgements: 0,
              active_turn: null,
              context_reset_at: null,
            },
            messages: [],
            acknowledgements: [],
    approvals: [],
            before_seq: null,
          },
        };
        this.rows.set(id, row);
      }
      if (row.history.conversation.active_turn) {
        return json({ message: "A turn is already active" }, 409);
      }
      const turn = crypto.randomUUID();
      const message = crypto.randomUUID();
      const block = `${message}-text`;
      const rebind = body.text.includes("reset context");
      row.notice = Boolean(row.history.conversation.context_reset_at) || rebind;
      if (rebind) row.history.conversation.context_reset_at = now;
      row.history.conversation.active_turn = { turn_id: turn, started_at: now, activities: [] };
      row.history.messages.push({
        id: crypto.randomUUID(),
        seq: row.history.messages.length + 1,
        turn_id: turn,
        role: "user",
        text: body.text,
        status: "completed",
        error_code: null,
        created_at: now,
        activities: [],
      });
      row.history.conversation.message_count = row.history.messages.length;
      this.prepareReply(row, body.text);
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
            if (current.history.conversation.active_turn) return;
            const reply = current.history.messages.at(-1)!;
            emit("block.completed", {
              block_id: block,
              block: { type: "text", block_id: block, text: reply.text },
            });
            emit("message.completed", { message_id: message });
            emit("turn.completed", {
              turn_id: turn,
              status: reply.error_code === "cancelled" ? "cancelled" : "completed",
              error: reply.error_code ? { code: "cancelled", message: "Stopped." } : null,
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
    return json({ message: "Fixture route not found" }, 404);
  };
}
