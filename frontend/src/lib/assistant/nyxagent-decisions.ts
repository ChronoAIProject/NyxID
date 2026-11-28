import { ApiError } from "@/lib/api-client";
import type { NyxAgentAcknowledgement, NyxAgentHistory } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { getAssistantIdentityRevision, subscribeAssistantIdentity } from "./identity";
import { continuationForAll, liveSend } from "./nyxagent-continuation";
import { nyxAgentTransport } from "./nyxagent-transport";

export interface AcknowledgementDecision {
  readonly id: string;
  readonly choice: "allow" | "deny";
  readonly acceptReducedScope?: boolean;
}

export interface DecisionSource {
  readonly conversationId: string | undefined;
  readonly owner: string | undefined;
  readonly identityRevision: number;
  readonly authorityRevision: number;
  readonly turnId: string | undefined;
  readonly messageSeq: number;
}

export function isCurrentDecisionActor(source: DecisionSource): boolean {
  return Boolean(source.owner) && source.owner === useAuthStore.getState().user?.id &&
    source.identityRevision === getAssistantIdentityRevision();
}

interface Continuation {
  readonly source: DecisionSource;
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly decisionHistory: NyxAgentHistory | undefined;
}

/** In-memory owner work survives view changes, but never an identity transition. */
class NyxAgentDecisions {
  private readonly queued = new Map<string, Continuation[]>();
  private readonly observers = new Map<string, number>();
  private readonly authorityRevisions = new Map<string, number>();
  private readonly reads = new Set<string>();
  private timer: ReturnType<typeof setInterval> | undefined;
  private draining = false;

  constructor() {
    nyxAgentTransport.subscribe(() => this.drain());
    subscribeAssistantIdentity(() => {
      this.queued.clear();
      this.authorityRevisions.clear();
      this.observers.clear();
      this.schedule();
    });
  }

  capture(conversationId: string | undefined, owner: string | undefined, identityRevision: number): DecisionSource {
    return {
      conversationId,
      owner,
      identityRevision,
      authorityRevision: conversationId ? this.authorityRevisions.get(conversationId) ?? 0 : 0,
      turnId: nyxAgentTransport.getActiveTurnId(conversationId),
      messageSeq: nyxAgentTransport.getHistory(conversationId)?.messages.at(-1)?.seq ?? 0,
    };
  }

  observe(conversationId: string): () => void {
    const revision = getAssistantIdentityRevision();
    this.observers.set(conversationId, (this.observers.get(conversationId) ?? 0) + 1);
    this.drain();
    return () => {
      if (revision !== getAssistantIdentityRevision()) return;
      const count = (this.observers.get(conversationId) ?? 1) - 1;
      if (count) this.observers.set(conversationId, count);
      else this.observers.delete(conversationId);
      this.schedule();
    };
  }

  refuse(conversationId: string): void {
    this.authorityRevisions.set(conversationId, (this.authorityRevisions.get(conversationId) ?? 0) + 1);
    this.queued.delete(conversationId);
    this.schedule();
  }

  refresh(): void { this.drain(); }

  isCurrentSource(source: DecisionSource): boolean {
    return isCurrentDecisionActor(source) && source.authorityRevision ===
      (this.authorityRevisions.get(source.conversationId ?? "") ?? 0);
  }

  private stopped(source: DecisionSource): boolean {
    return Boolean(source.turnId && nyxAgentTransport.getHistory(source.conversationId)?.messages.some((message) =>
      message.seq > source.messageSeq && message.turn_id === source.turnId &&
      message.role === "assistant" && message.status === "failed" && message.error_code === "cancelled",
    ));
  }

  complete(source: DecisionSource, choice: "allow" | "deny", acknowledgement: NyxAgentAcknowledgement): void {
    if (choice !== "allow" || !source.conversationId || !this.isCurrentSource(source) ||
      acknowledgement.continuation_owner === "server" || acknowledgement.decider === "orchestrator" ||
      acknowledgement.trigger_run_id || this.stopped(source)) return;
    const conversationId = source.conversationId;
    const entry = {
      source: { ...source, turnId: source.turnId ?? nyxAgentTransport.getActiveTurnId(conversationId) },
      acknowledgement,
      decisionHistory: nyxAgentTransport.getHistory(conversationId),
    };
    this.queued.set(conversationId, [...(this.queued.get(conversationId) ?? []), entry]);
    this.drain();
  }

  private drain(): void {
    if (this.draining) return;
    this.draining = true;
    try {
      for (const [conversationId, entries] of this.queued) {
        const valid = entries.filter(({ source, acknowledgement, decisionHistory }) => {
          if (!this.isCurrentSource(source) || this.stopped(source)) return false;
          // A settlement read may retire the first card while a later card is
          // still allowed. Select both text and marker from the surviving rows.
          const history = nyxAgentTransport.getHistory(conversationId);
          const row = history === decisionHistory ? acknowledgement :
            history?.acknowledgements.find((row) => row.id === acknowledgement.id);
          return row && (row.status === "allowed" || row.status === "used") &&
            row.decided_by === "user" && row.continuation_owner !== "server" &&
            row.decider !== "orchestrator" && !row.trigger_run_id &&
            Number.isFinite(Date.parse(row.expires_at)) &&
            // Backend expiry applies to allowed actions; persisted grants and
            // used receipts keep their decision after the pending deadline.
            (row.status === "used" || row.kind !== "action" || Date.parse(row.expires_at) > Date.now());
        });
        if (!valid.length) { this.queued.delete(conversationId); continue; }
        this.queued.set(conversationId, valid);
        if (nyxAgentTransport.isRunning(conversationId)) continue;
        this.queued.delete(conversationId);
        void liveSend(conversationId, continuationForAll(valid.map((entry) => entry.acknowledgement)),
          () => undefined, undefined, undefined, undefined, undefined, valid[0]!.acknowledgement.id,
        ).catch(() => undefined);
      }
    } finally {
      this.draining = false;
      this.schedule();
    }
  }

  private schedule(): void {
    const needed = [...this.queued.keys()].some((id) => !this.observers.has(id));
    if (!needed && this.timer !== undefined) { clearInterval(this.timer); this.timer = undefined; }
    if (needed && this.timer === undefined) this.timer = setInterval(() => this.poll(), 2000);
  }

  private poll(): void {
    for (const [conversationId, entries] of this.queued) {
      if (this.observers.has(conversationId)) continue;
      const source = entries[0]!.source;
      const readKey = `${source.identityRevision}:${conversationId}`;
      if (!this.isCurrentSource(source) || this.reads.has(readKey)) continue;
      this.reads.add(readKey);
      void nyxAgentTransport.history(conversationId).catch((error: unknown) => {
        if (this.isCurrentSource(source) && error instanceof ApiError && (error.status === 403 || error.status === 404)) {
          this.refuse(conversationId);
        }
      }).finally(() => this.reads.delete(readKey));
    }
  }
}

export const nyxAgentDecisions = new NyxAgentDecisions();
