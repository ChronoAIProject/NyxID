import { resolveAssistantAction } from "@/lib/assistant/action-registry";
import {
  assistantActionRequestSchema,
  findSecretPath,
  recoverUnsupportedAssistantActionRequest,
  type AssistantActionRequest,
} from "@/schemas/assistant-actions";

type JsonRecord = Record<string, unknown>;

export const ACTION_IDENTITY_KEYS = [
  "actorId",
  "originTurnId",
  "taskId",
  "stepId",
  "actionRequestId",
] as const;

export type ChatActionValidationResult =
  | {
      readonly request: AssistantActionRequest;
      readonly supported: true;
      readonly recovered: false;
    }
  | {
      readonly request: AssistantActionRequest;
      readonly supported: false;
      readonly recovered: true;
      readonly reason:
        | "invalid_shape"
        | "undeclared_field"
        | "unsupported_action";
    };

export class ChatActorProtocolError extends Error {
  readonly code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "ChatActorProtocolError";
    this.code = code;
  }
}

export function validateActionRequest(
  input: unknown,
): ChatActionValidationResult {
  const value = unpackAny(input);
  if (findSecretPath(value)) {
    throw new ChatActorProtocolError(
      "NyxID action input must not contain secrets.",
      "NYXID_SECRET_FORBIDDEN",
    );
  }

  const parsed = assistantActionRequestSchema.safeParse(value);
  if (!parsed.success) {
    return recoverActionRequest(
      value,
      parsed.error.issues.some(containsUnrecognizedKeys)
        ? "undeclared_field"
        : "invalid_shape",
    );
  }

  if (!resolveAssistantAction(parsed.data).supported) {
    return recoverActionRequest(value, "unsupported_action");
  }

  return { request: parsed.data, supported: true, recovered: false };
}

function recoverActionRequest(
  value: JsonRecord,
  reason: Extract<ChatActionValidationResult, { recovered: true }>["reason"],
): ChatActionValidationResult {
  const request = recoverUnsupportedAssistantActionRequest(value);
  if (!request) throw invalidVariant();
  return { request, supported: false, recovered: true, reason };
}

function containsUnrecognizedKeys(issue: unknown): boolean {
  if (!issue || typeof issue !== "object") return false;
  const record = issue as Record<string, unknown>;
  if (record.code === "unrecognized_keys") return true;
  return Object.values(record).some((value) =>
    Array.isArray(value) ? value.some(containsUnrecognizedKeys) : false,
  );
}

function unpackAny(input: unknown): JsonRecord {
  const value = optionalRecord(input);
  if (!value) throw invalidVariant();
  const nested = optionalRecord(value.value);
  if (nested) return nested;
  const result = { ...value };
  delete result["@type"];
  return result;
}

function optionalRecord(input: unknown): JsonRecord | null {
  return input && typeof input === "object" && !Array.isArray(input)
    ? (input as JsonRecord)
    : null;
}

function invalidVariant(): ChatActorProtocolError {
  return new ChatActorProtocolError(
    "NyxID action params are invalid.",
    "NYXID_ACTION_VARIANT_INVALID",
  );
}
