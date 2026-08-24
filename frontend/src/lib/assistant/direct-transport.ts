import {
  AssistantConversationNotFoundError,
  AssistantTurnActiveError,
} from "@/lib/assistant/errors";
import {
  getAssistantIdentityUserId,
  subscribeAssistantIdentity,
} from "@/lib/assistant/identity";
import { DIRECT_CONVERSATION_PREFIX } from "@/lib/assistant/conversation-ids";
import {
  isNyxidErrorEnvelope,
  parseJsonErrorResponse,
} from "@/lib/assistant/direct-http-error";
import { useAuthStore } from "@/stores/auth-store";
import type { Conversation } from "@/types/assistant";

const DIRECT_COMPLETIONS_URL = "/api/v1/assistant/direct/completions";
const DEFAULT_DIRECT_MODEL = "gpt-5.5";
const MAX_MESSAGE_CHARS = 32_768;
const MAX_OUTGOING_MESSAGES = 63;
const MAX_OUTGOING_CONTENT_BYTES = 256 * 1024;
const MAX_OUTGOING_REQUEST_BYTES = 256 * 1024;
const TITLE_CHARS = 40;
const FIRST_BYTE_TIMEOUT_MS = 30_000;
const IDLE_TIMEOUT_MS = 120_000;
const UTF8_ENCODER = new TextEncoder();

export type DirectTurnStatus =
  | "running"
  | "waiting"
  | "blocked"
  | "completed"
  | "failed"
  | "cancelled";

export interface DirectTextBlock {
  readonly type: "text";
  readonly block_id: string;
  readonly text: string;
}

export interface DirectAssistantMessage {
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly schema_version: 1;
  readonly blocks: DirectTextBlock[];
  readonly created_at: string;
}

interface DirectActiveTurn {
  readonly turnId: string | null;
  readonly status: DirectTurnStatus;
  readonly error: { readonly code: string; readonly message: string } | null;
}

interface DirectTurnState {
  readonly messages: DirectAssistantMessage[];
  readonly activeTurn: DirectActiveTurn | null;
  readonly lastCursor: number;
}

interface DirectTurnEventBase {
  readonly cursor: number;
}

export type DirectTurnEvent =
  | (DirectTurnEventBase & {
      readonly event: "turn.status";
      readonly turn_id: string;
      readonly status: DirectTurnStatus;
    })
  | (DirectTurnEventBase & {
      readonly event: "message.started";
      readonly message_id: string;
      readonly role: "assistant";
    })
  | (DirectTurnEventBase & {
      readonly event: "block.started";
      readonly message_id: string;
      readonly block_id: string;
      readonly index: number;
      readonly block: DirectTextBlock;
    })
  | (DirectTurnEventBase & {
      readonly event: "block.delta";
      readonly block_id: string;
      readonly text: string;
    })
  | (DirectTurnEventBase & {
      readonly event: "block.completed";
      readonly block_id: string;
      readonly block: DirectTextBlock;
    })
  | (DirectTurnEventBase & {
      readonly event: "message.completed";
      readonly message_id: string;
    })
  | (DirectTurnEventBase & {
      readonly event: "turn.completed";
      readonly turn_id: string | null;
      readonly status: "blocked" | "completed" | "failed" | "cancelled";
      readonly error: {
        readonly code: string;
        readonly message: string;
      } | null;
    });

export interface DirectTurnHandle {
  readonly turnId: string;
  cancel(): void;
}

export interface DirectConversationHistory {
  readonly conversation: Conversation;
  readonly messages: DirectAssistantMessage[];
  readonly activeTurn: DirectActiveTurn | null;
  readonly has_more: false;
}

const EMPTY_DIRECT_TURN_STATE: DirectTurnState = {
  messages: [],
  activeTurn: null,
  lastCursor: 0,
};

function isDirectTurnActive(status: DirectTurnStatus | undefined): boolean {
  return status === "running" || status === "waiting";
}

function applyDirectTurnEvent(
  state: DirectTurnState,
  event: DirectTurnEvent,
  receivedAt = new Date().toISOString(),
): DirectTurnState {
  if (event.cursor <= state.lastCursor) return state;
  const nextBase = { ...state, lastCursor: event.cursor };

  switch (event.event) {
    case "turn.status":
      return {
        ...nextBase,
        activeTurn: {
          turnId: event.turn_id,
          status: event.status,
          error: null,
        },
      };
    case "message.started":
      if (state.messages.some((message) => message.id === event.message_id)) {
        return nextBase;
      }
      return {
        ...nextBase,
        messages: [
          ...state.messages,
          {
            id: event.message_id,
            role: event.role,
            schema_version: 1,
            blocks: [],
            created_at: receivedAt,
          },
        ],
      };
    case "block.started":
      return {
        ...nextBase,
        messages: state.messages.map((message) => {
          if (message.id !== event.message_id) return message;
          const blocks = [...message.blocks];
          const existingIndex = blocks.findIndex(
            (block) => block.block_id === event.block_id,
          );
          if (existingIndex >= 0) blocks[existingIndex] = event.block;
          else blocks.splice(Math.min(event.index, blocks.length), 0, event.block);
          return { ...message, blocks };
        }),
      };
    case "block.delta":
      return {
        ...nextBase,
        messages: state.messages.map((message) => ({
          ...message,
          blocks: message.blocks.map((block) =>
            block.block_id === event.block_id
              ? { ...block, text: block.text + event.text }
              : block,
          ),
        })),
      };
    case "block.completed":
      return {
        ...nextBase,
        messages: state.messages.map((message) => ({
          ...message,
          blocks: message.blocks.map((block) =>
            block.block_id === event.block_id ? event.block : block,
          ),
        })),
      };
    case "message.completed":
      return nextBase;
    case "turn.completed":
      return {
        ...nextBase,
        activeTurn: {
          turnId: event.turn_id,
          status: event.status,
          error: event.error,
        },
      };
  }
}

function drainDirectSseBuffer(buffer: string): {
  readonly payloads: string[];
  readonly rest: string;
} {
  let working = buffer;
  let heldCr = "";
  if (working.endsWith("\r")) {
    heldCr = "\r";
    working = working.slice(0, -1);
  }
  const segments = working
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .split("\n\n");
  const rest = segments.pop() ?? "";
  const payloads = segments
    .map((segment) =>
      segment
        .split("\n")
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice("data:".length).trimStart())
        .join("\n"),
    )
    .filter(Boolean);
  return { payloads, rest: rest + heldCr };
}

function flushDirectSseBuffer(rest: string): string[] {
  return rest.trim() ? drainDirectSseBuffer(`${rest}\n\n`).payloads : [];
}

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
  readonly effort?: string;
}

export interface DirectConversationSettings {
  readonly model: string;
  readonly skillSlug: string | null;
  /** `null` sends no `effort`, leaving the upstream default in place. */
  readonly effort: string | null;
}

/**
 * One definition of "nothing chosen yet", shared by the draft seed, both
 * identity resets, and the unknown-conversation fallback — so a new setting
 * cannot be added to some of those four and forgotten in the rest.
 */
const DEFAULT_DIRECT_SETTINGS: DirectConversationSettings = {
  model: DEFAULT_DIRECT_MODEL,
  skillSlug: null,
  effort: null,
};

interface StoredConversation {
  conversation: Conversation;
  turnState: DirectTurnState;
  settings: DirectConversationSettings;
  modelSelected: boolean;
}

interface RunningTurn {
  readonly turnId: string;
  readonly controller: AbortController;
  readonly onEvent: (event: DirectTurnEvent) => void;
  cursor: number;
  currentMessageId: string | null;
  currentBlockId: string | null;
  accumulatedText: string;
  terminalEmitted: boolean;
  drained: boolean;
  discarded: boolean;
  sawFinishReason: boolean;
  sawUpstreamError: boolean;
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
  messages: readonly DirectAssistantMessage[],
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

function toBoundedDirectMessages(
  messages: readonly DirectAssistantMessage[],
): DirectMessage[] {
  const transcript = toDirectMessages(messages);
  const newestFirst: DirectMessage[] = [];
  let contentBytes = 0;

  for (
    let index = transcript.length - 1;
    index >= 0 && newestFirst.length < MAX_OUTGOING_MESSAGES;
    index -= 1
  ) {
    const message = transcript[index];
    if (!message) continue;
    const messageBytes = UTF8_ENCODER.encode(message.content).byteLength;
    if (contentBytes + messageBytes > MAX_OUTGOING_CONTENT_BYTES) break;
    newestFirst.push(message);
    contentBytes += messageBytes;
  }

  return newestFirst.reverse();
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

export class DirectAssistantTransport {
  private readonly conversations = new Map<string, StoredConversation>();
  private readonly running = new Map<string, RunningTurn>();
  private readonly settingsListeners = new Set<() => void>();
  private readonly stateListeners = new Set<() => void>();
  private readonly fetchFn: typeof fetch;
  private readonly firstByteTimeoutMs: number;
  private readonly idleTimeoutMs: number;
  private readonly now: () => number;
  private ownerUserId: string | null = getAssistantIdentityUserId();
  private draftSettings: DirectConversationSettings = {
    ...DEFAULT_DIRECT_SETTINGS,
  };
  private draftModelSelected = false;
  private revision = 0;

  constructor(options: DirectTransportOptions = {}) {
    // Wrap rather than alias the global: a detached `fetch` reference
    // invoked as `this.fetchFn(...)` runs with the transport as `this` and
    // real Chrome rejects that ("Illegal invocation"). Test-injected fetches
    // are used as-is.
    this.fetchFn = options.fetch ?? ((input, init) => fetch(input, init));
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
    this.draftSettings = { ...DEFAULT_DIRECT_SETTINGS };
    this.draftModelSelected = false;
    this.ownerUserId = userId;
    this.notifySettingsListeners();
    this.notifyStateListeners();
  }

  getOwnerUserId(): string | null {
    return this.ownerUserId;
  }

  getSettings(conversationId?: string): DirectConversationSettings {
    this.ensureOwner();
    if (!conversationId) return this.draftSettings;
    return this.conversations.get(conversationId)?.settings ?? DEFAULT_DIRECT_SETTINGS;
  }

  readonly subscribeSettings = (listener: () => void): (() => void) => {
    this.settingsListeners.add(listener);
    return () => this.settingsListeners.delete(listener);
  };

  readonly subscribeState = (listener: () => void): (() => void) => {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  };

  readonly getRevision = (): number => {
    this.ensureOwner();
    return this.revision;
  };

  getConversationsSnapshot(): readonly Conversation[] {
    this.ensureOwner();
    return [...this.conversations.values()]
      .map((stored) => stored.conversation)
      .sort((left, right) =>
        right.last_message_at.localeCompare(left.last_message_at),
      );
  }

  getHistorySnapshot(
    conversationId: string,
  ): DirectConversationHistory | null {
    this.ensureOwner();
    const stored = this.conversations.get(conversationId);
    if (!stored) return null;
    return {
      conversation: stored.conversation,
      messages: stored.turnState.messages,
      activeTurn: stored.turnState.activeTurn,
      has_more: false,
    };
  }

  canUpdateSettings(conversationId?: string): boolean {
    this.ensureOwner();
    if (!this.ownerUserId) return false;
    return !conversationId || this.conversations.has(conversationId);
  }

  setModel(conversationId: string | undefined, model: string): void {
    const applied = this.updateSettings(conversationId, { model });
    if (!applied) return;
    if (!conversationId) {
      this.draftModelSelected = true;
      return;
    }
    const stored = this.conversations.get(conversationId);
    if (stored) stored.modelSelected = true;
  }

  // Invoked during render by useDirectConversationSettings, so it must never
  // notify subscribers: a listener firing here means setState while another
  // component renders. The caller reads the snapshot after seeding in the same
  // render, and useSyncExternalStore's commit-time consistency check re-renders
  // any component that read the pre-seed snapshot earlier in that pass.
  seedDefaultModel(conversationId: string | undefined, model: string): void {
    this.ensureOwner();
    if (!this.ownerUserId) return;
    if (!conversationId) {
      if (this.draftModelSelected) return;
      if (this.draftSettings.model === model) return;
      this.draftSettings = { ...this.draftSettings, model };
      return;
    }
    const stored = this.conversations.get(conversationId);
    if (!stored) return;
    if (stored.modelSelected) return;
    if (stored.settings.model === model) return;
    stored.settings = { ...stored.settings, model };
    stored.conversation = { ...stored.conversation, llm_model: model };
  }

  setSkill(conversationId: string | undefined, skillSlug: string | null): void {
    this.updateSettings(conversationId, { skillSlug });
  }

  setEffort(conversationId: string | undefined, effort: string | null): void {
    this.updateSettings(conversationId, { effort });
  }

  async listConversations(): Promise<Conversation[]> {
    return [...this.getConversationsSnapshot()];
  }

  async createConversation(): Promise<Conversation> {
    this.ensureSignedInOwner();
    const createdAt = new Date(this.now()).toISOString();
    const conversation: Conversation = {
      id: `${DIRECT_CONVERSATION_PREFIX}${crypto.randomUUID()}`,
      title: "New chat",
      created_at: createdAt,
      last_message_at: createdAt,
      llm_model: this.draftSettings.model,
    };
    this.conversations.set(conversation.id, {
      conversation,
      turnState: EMPTY_DIRECT_TURN_STATE,
      settings: { ...this.draftSettings },
      modelSelected: this.draftModelSelected,
    });
    this.draftSettings = { ...DEFAULT_DIRECT_SETTINGS };
    this.draftModelSelected = false;
    this.notifyStateListeners();
    return conversation;
  }

  async getHistory(conversationId: string): Promise<DirectConversationHistory> {
    const history = this.getHistorySnapshot(conversationId);
    if (!history) throw new AssistantConversationNotFoundError();
    return history;
  }

  async deleteConversation(conversationId: string): Promise<void> {
    this.ensureOwner();
    this.cancelActiveTurn(conversationId);
    if (!this.conversations.delete(conversationId)) {
      throw new AssistantConversationNotFoundError();
    }
    this.notifyStateListeners();
  }

  sendMessage(
    conversationId: string,
    content: string,
    onEvent: (event: DirectTurnEvent) => void,
  ): DirectTurnHandle {
    this.ensureSignedInOwner();
    const stored = this.conversations.get(conversationId);
    if (!stored) throw new AssistantConversationNotFoundError();
    if (
      this.running.has(conversationId) ||
      isDirectTurnActive(stored.turnState.activeTurn?.status)
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

    let request: DirectRequest = {
      messages: toBoundedDirectMessages(stored.turnState.messages),
      model: stored.settings.model,
      ...(stored.settings.skillSlug
        ? { skill_slug: stored.settings.skillSlug }
        : {}),
      ...(stored.settings.effort ? { effort: stored.settings.effort } : {}),
    };
    while (
      request.messages.length > 1 &&
      UTF8_ENCODER.encode(JSON.stringify(request)).byteLength >
        MAX_OUTGOING_REQUEST_BYTES
    ) {
      request = { ...request, messages: request.messages.slice(1) };
    }
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
      sawUpstreamError: false,
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
  ): boolean {
    this.ensureOwner();
    // Picker controls can briefly outlive their memory-only conversation
    // during reload and identity transitions. Treat those writes as stale;
    // explicit conversation and turn operations remain strict.
    if (!this.ownerUserId) return false;
    if (!conversationId) {
      this.draftSettings = { ...this.draftSettings, ...patch };
      this.notifySettingsListeners();
      return true;
    }
    const stored = this.conversations.get(conversationId);
    if (!stored) return false;
    stored.settings = { ...stored.settings, ...patch };
    stored.conversation = {
      ...stored.conversation,
      llm_model: stored.settings.model,
    };
    this.notifySettingsListeners();
    this.notifyStateListeners();
    return true;
  }

  private notifySettingsListeners(): void {
    for (const listener of this.settingsListeners) listener();
  }

  private notifyStateListeners(): void {
    this.revision += 1;
    for (const listener of this.stateListeners) listener();
  }

  private nextCursor(run: RunningTurn): number {
    run.cursor += 1;
    return run.cursor;
  }

  private emit(
    conversationId: string,
    run: RunningTurn,
    event: DirectTurnEvent,
  ): void {
    if (run.discarded || run.drained) return;
    const stored = this.conversations.get(conversationId);
    if (stored) {
      stored.turnState = applyDirectTurnEvent(stored.turnState, event);
      stored.conversation = {
        ...stored.conversation,
        last_message_at: new Date(this.now()).toISOString(),
      };
    }
    if (event.event === "turn.completed") run.terminalEmitted = true;
    run.onEvent(event);
    this.notifyStateListeners();
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
        const body = await parseJsonErrorResponse(response);
        if (
          response.status >= 400 &&
          response.status < 500 &&
          isNyxidErrorEnvelope(body)
        ) {
          this.finishUi(conversationId, run, "failed", {
            code: `http_${String(response.status)}`,
            message: body.message,
          });
          this.finishDrain(conversationId, run);
          // Deliver the terminal event before invalidating identity. The auth
          // transition aborts and discards active direct runs, so clearing the
          // session first would swallow this failure from the UI.
          if (response.status === 401) {
            queueMicrotask(() => useAuthStore.getState().setUser(null));
          }
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
        const drained = drainDirectSseBuffer(buffer);
        buffer = drained.rest;
        for (const payload of drained.payloads) {
          if (this.handlePayload(conversationId, run, payload)) break;
        }
        if (run.sawDone || run.sawUpstreamError) break;
      }

      if (run.sawUpstreamError) {
        // The error event settles the UI, but the upstream may keep streaming.
        // Cancel before draining so the server-side in-flight permit is not
        // left to browser garbage collection.
        run.controller.abort();
        await reader.cancel().catch(() => undefined);
      }

      if (!run.sawDone && !run.sawUpstreamError) {
        buffer += decoder.decode();
        for (const payload of flushDirectSseBuffer(buffer)) {
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
      run.sawUpstreamError = true;
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
      this.openMessage(conversationId, run);
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

  private openMessage(conversationId: string, run: RunningTurn): void {
    if (run.currentBlockId) return;
    // OpenAI-compatible `id` values identify upstream responses, but they are
    // not a safe key for our in-memory transcript: gateways and deterministic
    // test fixtures may reuse one across requests. Every local turn must own a
    // distinct message/block identity or the reducer will treat later replies
    // as duplicate `message.started` events and discard their text deltas.
    const messageId = `${run.turnId}-assistant-message`;
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
            block: openBlock,
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
