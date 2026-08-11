import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
} from "@/lib/assistant/errors";
import {
  getAssistantIdentityUserId,
  subscribeAssistantIdentity,
} from "@/lib/assistant/identity";
import { drainSseBuffer, flushSseBuffer } from "@/lib/assistant/sse";
import {
  applyTurnEvent,
  EMPTY_TURN_STATE,
  toTerminalBlock,
} from "@/lib/assistant/stream";
import { useAuthStore } from "@/stores/auth-store";
import type {
  AssistantMessage,
  AssistantTransport,
  Conversation,
  ConversationHistory,
  ProjectionReconcileOutcome,
  TurnEvent,
  TurnHandle,
  TurnReducerState,
} from "@/types/assistant";
import { isTurnActive } from "@/types/assistant";
import type { ApiErrorResponse } from "@/types/api";

const DIRECT_COMPLETIONS_URL = "/api/v1/assistant/direct/completions";
const DEFAULT_DIRECT_MODEL = "gpt-5.5";
const MAX_MESSAGE_CHARS = 32_768;
const TITLE_CHARS = 40;
const FIRST_BYTE_TIMEOUT_MS = 30_000;
const IDLE_TIMEOUT_MS = 120_000;

interface CompletionChunk {
  readonly id?: string;
  readonly choices?: ReadonlyArray<{
    readonly delta?: { readonly content?: string | null };
    readonly finish_reason?: string | null;
  }>;
  readonly error?: { readonly code?: string; readonly message?: string };
}

interface DirectMessage {
  readonly role: "user" | "assistant";
  readonly content: string;
}

interface DirectRequest {
  readonly messages: readonly DirectMessage[];
  readonly model: string;
  readonly skill_slug?: string;
}

export interface DirectConversationSettings {
  readonly model: string;
  readonly skillSlug: string | null;
}

interface StoredConversation {
  conversation: Conversation;
  turnState: TurnReducerState;
  settings: DirectConversationSettings;
}

interface RunningTurn {
  readonly turnId: string;
  readonly controller: AbortController;
  readonly onEvent: (event: TurnEvent) => void;
  cursor: number;
  currentMessageId: string | null;
  currentBlockId: string | null;
  accumulatedText: string;
  terminalEmitted: boolean;
  drained: boolean;
  discarded: boolean;
  sawFinishReason: boolean;
  sawDone: boolean;
}

interface DirectTransportOptions {
  readonly fetch?: typeof fetch;
  readonly firstByteTimeoutMs?: number;
  readonly idleTimeoutMs?: number;
  readonly now?: () => number;
}

class DirectStreamTimeoutError extends Error {
  readonly code: "first_byte_timeout" | "idle_timeout";

  constructor(code: "first_byte_timeout" | "idle_timeout") {
    super(
      code === "first_byte_timeout"
        ? "The direct model did not start replying in time."
        : "The direct model stream stopped responding.",
    );
    this.name = "DirectStreamTimeoutError";
    this.code = code;
  }
}

function newId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

function toDirectMessages(
  messages: readonly AssistantMessage[],
): DirectMessage[] {
  const payload: DirectMessage[] = [];
  for (const message of messages) {
    if (message.role !== "user" && message.role !== "assistant") continue;
    const content = message.blocks
      .filter((block) => block.type === "text")
      .map((block) => (block.type === "text" ? block.text : ""))
      .filter(Boolean)
      .join("\n\n");
    if (content) payload.push({ role: message.role, content });
  }
  return payload;
}

function isNyxidErrorEnvelope(value: unknown): value is ApiErrorResponse {
  return Boolean(
    value &&
    typeof value === "object" &&
    "error" in value &&
    typeof value.error === "string" &&
    "error_code" in value &&
    typeof value.error_code === "number" &&
    "message" in value &&
    typeof value.message === "string",
  );
}

async function parseJsonError(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return null;
  }
}

function unavailableMessage(status: number): string {
  if (status === 401 || status === 403) {
    return "The direct model rejected NyxID's service credential. Reconnect the service and try again.";
  }
  if (status === 404) {
    return "Direct model chat is unavailable for this account.";
  }
  if (status === 429) {
    return "Direct model chat is busy. Wait a moment and try again.";
  }
  return "The direct model stream could not be started.";
}

export class DirectAssistantTransport implements AssistantTransport {
  private readonly conversations = new Map<string, StoredConversation>();
  private readonly running = new Map<string, RunningTurn>();
  private readonly fetchFn: typeof fetch;
  private readonly firstByteTimeoutMs: number;
  private readonly idleTimeoutMs: number;
  private readonly now: () => number;
  private ownerUserId: string | null = getAssistantIdentityUserId();
  private draftSettings: DirectConversationSettings = {
    model: DEFAULT_DIRECT_MODEL,
    skillSlug: null,
  };

  constructor(options: DirectTransportOptions = {}) {
    this.fetchFn = options.fetch ?? fetch;
    this.firstByteTimeoutMs =
      options.firstByteTimeoutMs ?? FIRST_BYTE_TIMEOUT_MS;
    this.idleTimeoutMs = options.idleTimeoutMs ?? IDLE_TIMEOUT_MS;
    this.now = options.now ?? Date.now;
    subscribeAssistantIdentity((userId) => this.resetForIdentity(userId));
  }

  resetForIdentity(userId: string | null): void {
    for (const run of this.running.values()) {
      run.discarded = true;
      run.controller.abort();
    }
    this.running.clear();
    this.conversations.clear();
    this.draftSettings = { model: DEFAULT_DIRECT_MODEL, skillSlug: null };
    this.ownerUserId = userId;
  }

  getOwnerUserId(): string | null {
    return this.ownerUserId;
  }

  getSettings(conversationId?: string): DirectConversationSettings {
    this.ensureOwner();
    if (!conversationId) return this.draftSettings;
    return (
      this.conversations.get(conversationId)?.settings ?? {
        model: DEFAULT_DIRECT_MODEL,
        skillSlug: null,
      }
    );
  }

  setModel(conversationId: string | undefined, model: string): void {
    this.updateSettings(conversationId, { model });
  }

  setSkill(conversationId: string | undefined, skillSlug: string | null): void {
    this.updateSettings(conversationId, { skillSlug });
  }

  async listConversations(): Promise<Conversation[]> {
    this.ensureOwner();
    return [...this.conversations.values()]
      .map((stored) => stored.conversation)
      .sort((left, right) =>
        right.last_message_at.localeCompare(left.last_message_at),
      );
  }

  async createConversation(): Promise<Conversation> {
    this.ensureSignedInOwner();
    const createdAt = new Date(this.now()).toISOString();
    const conversation: Conversation = {
      id: newId("direct"),
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
      llm_model: this.draftSettings.model,
    };
    this.conversations.set(conversation.id, {
      conversation,
      turnState: EMPTY_TURN_STATE,
      settings: { ...this.draftSettings },
    });
    this.draftSettings = { model: DEFAULT_DIRECT_MODEL, skillSlug: null };
    return conversation;
  }

  async getHistory(conversationId: string): Promise<ConversationHistory> {
    this.ensureOwner();
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    return {
      conversation: stored.conversation,
      messages: stored.turnState.messages,
      has_more: false,
    };
  }

  async reconcileProjection(
    conversationId: string,
  ): Promise<ProjectionReconcileOutcome> {
    this.ensureOwner();
    return this.conversations.has(conversationId)
      ? { status: "materialized", conversationId }
      : { status: "absent", conversationId };
  }

  releaseProjectionWaiter(conversationId: string): void {
    void conversationId;
  }

  async deleteConversation(conversationId: string): Promise<void> {
    this.ensureOwner();
    this.cancelActiveTurn(conversationId);
    if (!this.conversations.delete(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: TurnEvent) => void,
  ): TurnHandle {
    this.ensureSignedInOwner();
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    if (
      this.running.has(conversationId) ||
      isTurnActive(stored.turnState.activeTurn?.status)
    ) {
      throw new AssistantTurnActiveError();
    }

    const normalized = content.trim();
    if (!normalized || [...normalized].length > MAX_MESSAGE_CHARS) {
      throw new Error("Message must contain between 1 and 32768 characters.");
    }

    const createdAt = new Date(this.now()).toISOString();
    const firstMessage = stored.turnState.messages.length === 0;
    stored.turnState = {
      ...stored.turnState,
      messages: [
        ...stored.turnState.messages,
        {
          id: newId("user-message"),
          role: "user",
          schema_version: 1,
          blocks: [
            {
              type: "text",
              block_id: newId("user-block"),
              text: normalized,
            },
          ],
          created_at: createdAt,
        },
      ],
      lastCursor: 0,
    };
    stored.conversation = {
      ...stored.conversation,
      title: firstMessage
        ? [...normalized].slice(0, TITLE_CHARS).join("")
        : stored.conversation.title,
      last_message_at: createdAt,
      llm_model: stored.settings.model,
    };

    const request: DirectRequest = {
      messages: toDirectMessages(stored.turnState.messages),
      model: stored.settings.model,
      ...(stored.settings.skillSlug
        ? { skill_slug: stored.settings.skillSlug }
        : {}),
    };
    const run: RunningTurn = {
      turnId: newId("turn"),
      controller: new AbortController(),
      onEvent,
      cursor: 0,
      currentMessageId: null,
      currentBlockId: null,
      accumulatedText: "",
      terminalEmitted: false,
      drained: false,
      discarded: false,
      sawFinishReason: false,
      sawDone: false,
    };
    this.running.set(conversationId, run);
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.status",
      turn_id: run.turnId,
      status: "running",
    });
    void this.streamTurn(conversationId, run, request);
    return {
      turnId: run.turnId,
      cancel: () => this.cancelRun(conversationId, run),
    };
  }

  cancelActiveTurn(conversationId: string): void {
    const run = this.running.get(conversationId);
    if (run) this.cancelRun(conversationId, run);
  }

  async decideApproval(): Promise<TurnHandle | null> {
    throw new Error("Approvals are not available in direct model chat.");
  }

  setActionCardInProgress(): void {
    throw new Error("Actions are not available in direct model chat.");
  }

  blockActionCard(): void {
    throw new Error("Actions are not available in direct model chat.");
  }

  continueActions(): TurnHandle | null {
    throw new Error("Actions are not available in direct model chat.");
  }

  wakeActions(): TurnHandle {
    throw new Error("Actions are not available in direct model chat.");
  }

  private ensureOwner(): void {
    const current = getAssistantIdentityUserId();
    if (current !== this.ownerUserId) this.resetForIdentity(current);
  }

  private ensureSignedInOwner(): void {
    this.ensureOwner();
    if (!this.ownerUserId) throw new Error("Sign in to use direct model chat.");
  }

  private updateSettings(
    conversationId: string | undefined,
    patch: Partial<DirectConversationSettings>,
  ): void {
    this.ensureSignedInOwner();
    if (!conversationId) {
      this.draftSettings = { ...this.draftSettings, ...patch };
      return;
    }
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    stored.settings = { ...stored.settings, ...patch };
    stored.conversation = {
      ...stored.conversation,
      llm_model: stored.settings.model,
    };
  }

  private nextCursor(run: RunningTurn): number {
    run.cursor += 1;
    return run.cursor;
  }

  private emit(
    conversationId: string,
    run: RunningTurn,
    event: TurnEvent,
  ): void {
    if (run.discarded || run.drained) return;
    const stored = this.conversations.get(conversationId);
    if (stored) {
      stored.turnState = applyTurnEvent(stored.turnState, event);
      stored.conversation = {
        ...stored.conversation,
        last_message_at: new Date(this.now()).toISOString(),
      };
    }
    if (event.event === "turn.completed") run.terminalEmitted = true;
    run.onEvent(event);
  }

  private async withTimeout<T>(
    task: Promise<T>,
    timeoutMs: number,
    run: RunningTurn,
    code: "first_byte_timeout" | "idle_timeout",
  ): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([
        task,
        new Promise<T>((_resolve, reject) => {
          timer = setTimeout(
            () => {
              reject(new DirectStreamTimeoutError(code));
              run.controller.abort();
            },
            Math.max(0, timeoutMs),
          );
        }),
      ]);
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }

  private async streamTurn(
    conversationId: string,
    run: RunningTurn,
    request: DirectRequest,
  ): Promise<void> {
    const firstByteDeadline = this.now() + this.firstByteTimeoutMs;
    try {
      const response = await this.withTimeout(
        this.fetchFn(DIRECT_COMPLETIONS_URL, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Accept: "text/event-stream",
          },
          credentials: "include",
          body: JSON.stringify(request),
          signal: run.controller.signal,
        }),
        this.firstByteTimeoutMs,
        run,
        "first_byte_timeout",
      );
      if (!response.ok) {
        const body = await parseJsonError(response);
        if (response.status === 401 && isNyxidErrorEnvelope(body)) {
          useAuthStore.getState().setUser(null);
          this.finishUi(conversationId, run, "failed", {
            code: `http_${String(response.status)}`,
            message: body.message,
          });
          this.finishDrain(conversationId, run);
          return;
        }
        this.finishUi(conversationId, run, "failed", {
          code: `http_${String(response.status)}`,
          message: unavailableMessage(response.status),
        });
        this.finishDrain(conversationId, run);
        return;
      }
      if (!response.body) {
        this.finishUi(conversationId, run, "failed", {
          code: "missing_stream",
          message: "The direct model returned no stream.",
        });
        this.finishDrain(conversationId, run);
        return;
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      let firstRead = true;
      for (;;) {
        const timeout = firstRead
          ? Math.max(0, firstByteDeadline - this.now())
          : this.idleTimeoutMs;
        const result = await this.withTimeout(
          reader.read(),
          timeout,
          run,
          firstRead ? "first_byte_timeout" : "idle_timeout",
        );
        firstRead = false;
        if (result.done) break;
        buffer += decoder.decode(result.value, { stream: true });
        const drained = drainSseBuffer(buffer);
        buffer = drained.rest;
        for (const payload of drained.payloads) {
          if (this.handlePayload(conversationId, run, payload)) break;
        }
        if (run.sawDone) break;
      }

      if (!run.sawDone) {
        buffer += decoder.decode();
        for (const payload of flushSseBuffer(buffer)) {
          this.handlePayload(conversationId, run, payload);
        }
      }
      if (!run.terminalEmitted) {
        this.closeOpenMessage(conversationId, run);
        if (run.sawFinishReason || run.sawDone) {
          this.finishUi(conversationId, run, "completed", null);
        } else {
          this.finishUi(conversationId, run, "failed", {
            code: "truncated_stream",
            message: "The direct model stream ended before completion.",
          });
        }
      }
      this.finishDrain(conversationId, run);
    } catch (error) {
      if (run.discarded || run.drained) return;
      if (run.controller.signal.aborted && run.terminalEmitted) {
        this.finishDrain(conversationId, run);
        return;
      }
      this.closeOpenMessage(conversationId, run);
      const timeout =
        error instanceof DirectStreamTimeoutError ? error : undefined;
      this.finishUi(conversationId, run, "failed", {
        code: timeout?.code ?? "network_error",
        message:
          timeout?.message ??
          (error instanceof Error ? error.message : "The stream failed."),
      });
      this.finishDrain(conversationId, run);
    }
  }

  private handlePayload(
    conversationId: string,
    run: RunningTurn,
    payload: string,
  ): boolean {
    if (payload === "[DONE]") {
      run.sawDone = true;
      this.closeOpenMessage(conversationId, run);
      this.finishUi(conversationId, run, "completed", null);
      return true;
    }

    let chunk: CompletionChunk;
    try {
      chunk = JSON.parse(payload) as CompletionChunk;
    } catch {
      return false;
    }
    if (chunk.error) {
      this.closeOpenMessage(conversationId, run);
      this.finishUi(conversationId, run, "failed", {
        code: chunk.error.code ?? "direct_model_error",
        message: chunk.error.message ?? "The direct model run failed.",
      });
      return true;
    }

    const choice = chunk.choices?.[0];
    const delta = choice?.delta?.content;
    if (delta && !run.sawFinishReason) {
      this.openMessage(conversationId, run, chunk.id);
      run.accumulatedText += delta;
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "block.delta",
        block_id: run.currentBlockId!,
        text: delta,
      });
    }
    if (choice?.finish_reason) {
      run.sawFinishReason = true;
      this.closeOpenMessage(conversationId, run);
      // Visible completion is independent from draining the reader. The run
      // remains in `running` until usage and [DONE]/EOF have been consumed.
      this.finishUi(conversationId, run, "completed", null);
    }
    return false;
  }

  private openMessage(
    conversationId: string,
    run: RunningTurn,
    upstreamMessageId?: string,
  ): void {
    if (run.currentBlockId) return;
    const messageId = upstreamMessageId ?? newId("assistant-message");
    run.currentMessageId = messageId;
    run.currentBlockId = `${messageId}-text`;
    run.accumulatedText = "";
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "message.started",
      message_id: messageId,
      role: "assistant",
    });
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.started",
      message_id: messageId,
      block_id: run.currentBlockId,
      index: 0,
      block: { type: "text", block_id: run.currentBlockId, text: "" },
    });
  }

  private closeOpenMessage(conversationId: string, run: RunningTurn): void {
    if (!run.currentMessageId || !run.currentBlockId) return;
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "block.completed",
      block_id: run.currentBlockId,
      block: {
        type: "text",
        block_id: run.currentBlockId,
        text: run.accumulatedText,
      },
    });
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "message.completed",
      message_id: run.currentMessageId,
    });
    run.currentMessageId = null;
    run.currentBlockId = null;
    run.accumulatedText = "";
  }

  private finishUi(
    conversationId: string,
    run: RunningTurn,
    status: "completed" | "failed" | "cancelled",
    error: { readonly code: string; readonly message: string } | null,
  ): void {
    if (run.terminalEmitted) return;
    this.emit(conversationId, run, {
      cursor: this.nextCursor(run),
      event: "turn.completed",
      turn_id: run.turnId,
      status,
      error,
    });
  }

  private finishDrain(conversationId: string, run: RunningTurn): void {
    if (run.drained) return;
    run.drained = true;
    if (this.running.get(conversationId) === run) {
      this.running.delete(conversationId);
    }
  }

  private cancelRun(conversationId: string, run: RunningTurn): void {
    if (run.discarded || run.drained) return;
    run.controller.abort();
    if (!run.terminalEmitted) {
      if (run.currentBlockId) {
        const stored = this.conversations.get(conversationId);
        const openBlock = stored?.turnState.messages
          .flatMap((message) => message.blocks)
          .find((block) => block.block_id === run.currentBlockId);
        if (openBlock) {
          this.emit(conversationId, run, {
            cursor: this.nextCursor(run),
            event: "block.completed",
            block_id: run.currentBlockId,
            block: toTerminalBlock(openBlock),
          });
        }
        if (run.currentMessageId) {
          this.emit(conversationId, run, {
            cursor: this.nextCursor(run),
            event: "message.completed",
            message_id: run.currentMessageId,
          });
        }
        run.currentMessageId = null;
        run.currentBlockId = null;
      }
      this.emit(conversationId, run, {
        cursor: this.nextCursor(run),
        event: "turn.status",
        turn_id: run.turnId,
        status: "cancelled",
      });
      this.finishUi(conversationId, run, "cancelled", null);
    }
    this.finishDrain(conversationId, run);
  }
}

export const directAssistantTransport = new DirectAssistantTransport();
