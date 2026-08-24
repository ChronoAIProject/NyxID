import {
  ACTION_REGISTRY,
  resolveAssistantAction,
} from "@/lib/assistant/action-registry";
import {
  ACTION_SCHEMA_VERSION,
  actionControlIdentitySchema,
  assistantActionRequestSchema,
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

const ACTION_KEYS = [
  "schemaVersion",
  ...ACTION_IDENTITY_KEYS,
  "action",
  "params",
] as const;

export class ChatActorProtocolError extends Error {
  readonly code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "ChatActorProtocolError";
    this.code = code;
  }
}

export function validateActionRequest(input: unknown): AssistantActionRequest {
  const value = unpackAny(input);
  assertAllowedKeys(value, ACTION_KEYS);
  for (const key of ACTION_IDENTITY_KEYS) {
    if (!actionControlIdentitySchema.safeParse(value[key]).success) {
      throw new ChatActorProtocolError(
        "NyxID action identity is invalid.",
        "NYXID_IDENTITY_INVALID",
      );
    }
  }

  const parsed = assistantActionRequestSchema.safeParse(value);
  if (!parsed.success) {
    const issues = JSON.stringify(parsed.error.issues);
    if (issues.includes("unrecognized_keys")) {
      throw new ChatActorProtocolError(
        "NyxID action contains an undeclared field.",
        "NYXID_FIELD_UNDECLARED",
      );
    }
    if (issues.includes("Action request contained secret material")) {
      throw new ChatActorProtocolError(
        "NyxID action input must not contain secrets.",
        "NYXID_SECRET_FORBIDDEN",
      );
    }
    throw invalidVariant();
  }

  const resolved = resolveAssistantAction(parsed.data);
  const registered = ACTION_REGISTRY[parsed.data.action];
  if (
    parsed.data.schemaVersion === ACTION_SCHEMA_VERSION &&
    registered?.wiring !== "deferred" &&
    !resolved.supported
  ) {
    if (parsed.data.action === "service.connect" && hasUnsafeEndpoint(value)) {
      throw new ChatActorProtocolError(
        "NyxID action URL is unsafe.",
        "NYXID_URL_UNSAFE",
      );
    }
    throw invalidVariant();
  }

  return parsed.data;
}

function hasUnsafeEndpoint(value: JsonRecord): boolean {
  const params = optionalRecord(value.params);
  const custom = optionalRecord(params?.customService);
  if (!custom || typeof custom.endpointUrl !== "string") return false;
  try {
    const url = new URL(custom.endpointUrl);
    return (
      url.protocol !== "https:" ||
      !url.hostname ||
      Boolean(url.username || url.password || url.search || url.hash)
    );
  } catch {
    return true;
  }
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

function assertAllowedKeys(
  value: JsonRecord,
  allowed: readonly string[],
): void {
  const declared = new Set(allowed);
  if (Object.keys(value).some((key) => !declared.has(key))) {
    throw new ChatActorProtocolError(
      "NyxID action contains an undeclared field.",
      "NYXID_FIELD_UNDECLARED",
    );
  }
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
