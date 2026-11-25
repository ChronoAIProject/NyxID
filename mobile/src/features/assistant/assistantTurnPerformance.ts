import { sanitizeAssistantMessageContent } from "./assistantChatContent";
import { liveReplyText, type AssistantLiveReply } from "./assistantStream";
import type { AssistantTurnEvent } from "./assistantTypes";

type Outcome = "completed" | "no_text" | "cancelled" | "error" | "timeout";
export type ChatTurnTimings = {
  outcome: Outcome;
  duration_ms: number;
  headers_ms?: number;
  first_text_ms?: number;
  first_text_commit_ms?: number;
};

// A terminal frame and its first visible text can arrive in the same JS task.
// Allow React to commit the streaming or persisted reply after that frame.
export const CHAT_COMMIT_GRACE_MS = 2000;

export class AssistantTurnPerformance {
  private readonly startedAt: number;
  private turnId?: string;
  private headers?: number;
  private firstText?: number;
  private firstCommit?: number;
  private terminal?: Pick<ChatTurnTimings, "outcome" | "duration_ms">;
  private submitted = false;
  private timer?: ReturnType<typeof setTimeout>;

  constructor(
    private readonly submit: (timings: ChatTurnTimings) => void,
    private readonly now = () => performance.now(),
  ) {
    this.startedAt = now();
  }

  private elapsed() {
    return Math.round(this.now() - this.startedAt);
  }

  admitted() {
    if (!this.terminal) this.headers ??= this.elapsed();
  }

  observe(event: AssistantTurnEvent, reply: AssistantLiveReply) {
    if (this.terminal) return;
    if (event.event === "turn.status") this.turnId = reply.turnId;
    if (reply.terminal) {
      this.finish(reply.terminal === "failed" ? "error" : reply.terminal);
    } else if (
      this.firstText === undefined &&
      (event.event === "block.started" ||
        event.event === "block.delta" ||
        event.event === "block.completed") &&
      sanitizeAssistantMessageContent(liveReplyText(reply))
    ) {
      this.firstText = this.elapsed();
    }
  }

  commitFor(turnId?: string | null) {
    if (
      !turnId ||
      turnId !== this.turnId ||
      this.submitted ||
      this.firstCommit !== undefined
    )
      return undefined;
    return this.commit;
  }

  private commit = () => {
    if (this.submitted || this.firstCommit !== undefined) return;
    this.firstCommit = this.elapsed();
    if (this.terminal) this.flush();
  };

  finish(outcome: Outcome) {
    if (this.terminal) return;
    const hasText =
      this.firstText !== undefined || this.firstCommit !== undefined;
    this.terminal = {
      outcome: outcome === "completed" && !hasText ? "no_text" : outcome,
      duration_ms: this.elapsed(),
    };
    if (hasText && this.firstCommit === undefined)
      this.timer = setTimeout(() => this.flush(), CHAT_COMMIT_GRACE_MS);
    else this.flush();
  }

  private flush() {
    if (this.submitted || !this.terminal) return;
    this.submitted = true;
    clearTimeout(this.timer);
    this.turnId = undefined;
    this.submit({
      ...this.terminal,
      ...(this.headers !== undefined ? { headers_ms: this.headers } : {}),
      ...(this.firstText !== undefined
        ? { first_text_ms: this.firstText }
        : {}),
      ...(this.firstCommit !== undefined
        ? { first_text_commit_ms: this.firstCommit }
        : {}),
    });
  }
}
