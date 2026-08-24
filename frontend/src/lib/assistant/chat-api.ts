import { ApiError } from "@/lib/api-client";
import type {
  ChatTarget,
  ChatUsageSummary,
} from "@/lib/assistant/chat-types";
import {
  normalizeBackendSseFrame,
  SsePayloadDecoder,
} from "@/lib/assistant/sse-frame-normalizer";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import type { AGUIEvent } from "@/lib/assistant/agui-types";
import type { ActionReport } from "@/schemas/assistant-actions";

type JsonRecord = Record<string, unknown>;

export interface ChatStreamFrame {
  readonly event: AGUIEvent | null;
  readonly raw: unknown;
}

export type ChatInputAnswer =
  | { readonly freeText: string }
  | { readonly selectedOptionIds: readonly string[] };

export type ChatCommand =
  | {
      readonly type: "text";
      readonly clientRequestId: string;
      readonly conversationId?: string;
      readonly prompt: string;
    }
  | {
      readonly type: "plan.resolve";
      readonly conversationId: string;
      readonly taskId: string;
      readonly planId: string;
      readonly requestId: string;
      readonly clientRequestId: string;
      readonly planRevision: number;
      readonly confirmed: boolean;
      readonly expectedStateVersion: number;
    }
  | {
      readonly type: "input.resolve";
      readonly conversationId: string;
      readonly requestId: string;
      readonly clientRequestId: string;
      readonly answer: ChatInputAnswer;
      readonly expectedStateVersion: number;
    }
  | {
      readonly type: "approval.resolve";
      readonly conversationId: string;
      readonly requestId: string;
      readonly clientRequestId: string;
      readonly approved: boolean;
      readonly reason?: string;
      readonly expectedStateVersion: number;
    }
  | {
      readonly type: "task.stop";
      readonly conversationId: string;
      readonly turnId: string;
      readonly stopRequestId: string;
      readonly clientRequestId: string;
      readonly expectedStateVersion: number;
    }
  | {
      readonly type: "task.steer";
      readonly conversationId: string;
      readonly turnId: string;
      readonly steeringId: string;
      readonly clientRequestId: string;
      readonly instruction: string;
      readonly expectedStateVersion: number;
    }
  | {
      readonly type: "step.retry" | "step.skip";
      readonly conversationId: string;
      readonly turnId: string;
      readonly taskId: string;
      readonly stepId: string;
      readonly retryRequestId?: string;
      readonly skipRequestId?: string;
      readonly clientRequestId: string;
      readonly expectedOperationGeneration: number;
      readonly expectedStateVersion: number;
    }
  | {
      readonly type: "action.continue";
      readonly conversationId: string;
      readonly originTurnId?: string;
      readonly clientRequestId: string;
      readonly actions: readonly ActionReport[];
    };

export class ChatApiError extends Error {
  readonly code?: string | number;
  readonly status: number;

  constructor(message: string, status: number, code?: string | number) {
    super(message);
    this.name = "ChatApiError";
    this.code = code;
    this.status = status;
  }
}

function compactObject<T extends Record<string, unknown>>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T;
}

export async function sendChatCommand(
  command: ChatCommand,
  signal: AbortSignal,
): Promise<Response> {
  const clientRequestId = command.clientRequestId.trim();
  const body = compactObject(
    Object.fromEntries(
      Object.entries(command).map(([key, value]) => [
        key,
        typeof value === "string" ? value.trim() : value,
      ]),
    ),
  );
  try {
    return await assistantHttp("/assistant/chat", {
      body,
      headers: {
        Accept:
          command.type === "text" || command.type === "action.continue"
            ? "text/event-stream"
            : "application/json",
        "Idempotency-Key": clientRequestId,
      },
      method: "POST",
      signal,
    });
  } catch (error) {
    if (error instanceof ApiError) {
      throw new ChatApiError(
        error.message,
        error.status,
        error.errorCode >= 0 ? error.errorCode : error.errorResponse.error,
      );
    }
    throw error;
  }
}

export async function* readChatStreamFrames(
  response: Response,
  options?: { readonly signal?: AbortSignal },
): AsyncGenerator<ChatStreamFrame, void, undefined> {
  if (!response.body) throw new Error("Chat response has no readable stream.");
  const reader = response.body.getReader();
  const decoder = new SsePayloadDecoder();
  try {
    while (!options?.signal?.aborted) {
      const { done, value } = await readWithSignal(reader, options?.signal);
      const payloads = done ? decoder.finish() : decoder.push(value);
      for (const payload of payloads) {
        const data = payload.trim();
        if (!data || data === "[DONE]") continue;
        try {
          const raw: unknown = JSON.parse(data);
          yield { event: normalizeBackendSseFrame(raw), raw };
        } catch {
          // A malformed frame does not invalidate later frames in the stream.
        }
      }
      if (done) break;
    }
  } finally {
    reader.releaseLock();
  }
}

function readWithSignal(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  signal?: AbortSignal,
): Promise<ReadableStreamReadResult<Uint8Array>> {
  if (!signal) return reader.read();
  if (signal.aborted) return Promise.reject(signal.reason);
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason);
    signal.addEventListener("abort", abort, { once: true });
    void reader.read().then(resolve, reject).finally(() => {
      signal.removeEventListener("abort", abort);
    });
  });
}

function asRecord(value: unknown): JsonRecord | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : undefined;
}

function readString(record: JsonRecord | undefined, ...keys: string[]): string {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return "";
}

function readNumber(
  record: JsonRecord | undefined,
  ...keys: string[]
): number | undefined {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "string" && value.trim()) {
      const parsed = Number(value);
      if (Number.isFinite(parsed)) return parsed;
    }
  }
  return undefined;
}

function normalizeUsage(record: JsonRecord | undefined): ChatUsageSummary | null {
  if (!record) return null;
  const usage: ChatUsageSummary = {
    completionTokens: readNumber(record, "completionTokens", "completion_tokens"),
    cost: readNumber(record, "cost"),
    latencyMs: readNumber(record, "latencyMs", "latency_ms"),
    model: readString(record, "model") || undefined,
    promptTokens: readNumber(record, "promptTokens", "prompt_tokens"),
    totalTokens: readNumber(record, "totalTokens", "total_tokens"),
  };
  return Object.values(usage).some((value) => value !== undefined && value !== "")
    ? usage
    : null;
}

function normalizeTarget(record: JsonRecord | undefined): ChatTarget | null {
  if (!record) return null;
  const target: ChatTarget = {
    memberId: readString(record, "memberId", "member_id") || undefined,
    runId:
      readString(record, "runId", "run_id", "actorId", "actor_id") ||
      undefined,
    scopeId: readString(record, "scopeId", "scope_id") || undefined,
    studioUrl: readString(record, "studioUrl", "studio_url") || undefined,
    teamId: readString(record, "teamId", "team_id") || undefined,
    workflowId: readString(record, "workflowId", "workflow_id") || undefined,
  };
  return Object.values(target).some(Boolean) ? target : null;
}

function unpackStruct(value: unknown): JsonRecord | undefined {
  const record = asRecord(value);
  if (!record) return undefined;
  const fields = asRecord(record.fields);
  if (!fields) return record;
  const unpacked: JsonRecord = {};
  for (const [key, raw] of Object.entries(fields)) {
    const field = asRecord(raw);
    if (typeof field?.stringValue === "string") unpacked[key] = field.stringValue;
    else if (typeof field?.numberValue === "number") unpacked[key] = field.numberValue;
    else if (typeof field?.boolValue === "boolean") unpacked[key] = field.boolValue;
    else if (field?.structValue) unpacked[key] = unpackStruct(field.structValue);
  }
  return unpacked;
}

function merge<T extends object>(current: T | undefined, next: T | null): T | undefined {
  return next
    ? ({
        ...current,
        ...Object.fromEntries(
          Object.entries(next).filter(([, value]) => value !== undefined && value !== ""),
        ),
      } as T)
    : current;
}

export function extractChatStreamArtifacts(frames: readonly unknown[]): {
  readonly target?: ChatTarget;
  readonly usage?: ChatUsageSummary;
} {
  let target: ChatTarget | undefined;
  let usage: ChatUsageSummary | undefined;
  for (const raw of frames) {
    const frame = asRecord(raw);
    if (!frame) continue;
    usage = merge(usage, normalizeUsage(asRecord(frame.usage)));
    target = merge(target, normalizeTarget(frame));
    const result = asRecord(asRecord(frame.runFinished)?.result);
    usage = merge(usage, normalizeUsage(asRecord(result?.usage)));
    target = merge(target, normalizeTarget(result));
    const payload = unpackStruct(asRecord(frame.custom)?.payload);
    usage = merge(usage, normalizeUsage(asRecord(payload?.usage)));
    target = merge(target, normalizeTarget(payload));
    const observed = asRecord(payload?.payload);
    usage = merge(usage, normalizeUsage(asRecord(observed?.usage)));
    target = merge(target, normalizeTarget(observed));
  }
  return compactObject({ target, usage });
}
