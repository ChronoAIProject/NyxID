import {
  humanizeAssistantServiceSlug,
  safeAssistantDisplayText,
} from "@/lib/assistant/chat-display-safety";
import type { ConnectCardContentBlock } from "@/types/assistant";

type JsonRecord = Record<string, unknown>;

export type ChatAuthorizationReasonCode =
  | "NYXID_SERVICE_NOT_CONNECTED"
  | "NYXID_UNAUTHORIZED";

export interface ChatAuthorizationBlocker {
  readonly serviceSlug: string;
  readonly serviceLabel: string;
  readonly reasonCode: ChatAuthorizationReasonCode;
  readonly safeMessage: string;
}

function asRecord(value: unknown): JsonRecord | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function unpackAny(value: unknown): JsonRecord {
  const record = asRecord(value);
  if (!record) return {};
  const nested = asRecord(record.value);
  if (nested) return nested;
  const result = { ...record };
  delete result["@type"];
  return result;
}

export function parseAuthorizationBlocker(
  value: unknown,
): ChatAuthorizationBlocker | null {
  const record = unpackAny(value);
  const reasonCode = record.reasonCode;
  if (
    reasonCode !== "NYXID_SERVICE_NOT_CONNECTED" &&
    reasonCode !== "NYXID_UNAUTHORIZED"
  ) {
    return null;
  }
  if (typeof record.serviceSlug !== "string") return null;
  const serviceSlug = record.serviceSlug.trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9-]{0,127}$/.test(serviceSlug)) return null;

  const serviceLabel = safeAssistantDisplayText(
    record.serviceLabel,
    humanizeAssistantServiceSlug(serviceSlug),
    128,
  );
  return {
    reasonCode,
    safeMessage: safeAssistantDisplayText(
      record.safeMessage,
      `Connect or reauthorize ${serviceSlug} to continue.`,
    ),
    serviceLabel,
    serviceSlug,
  };
}

const REQUIRE_SERVICE_BLOCKED_STATUS = "ServiceRegistrationRequired";

export function parseToolResultBlocker(
  result: unknown,
): ChatAuthorizationBlocker | null {
  if (typeof result !== "string" || !result.includes("blocked")) return null;
  let record: JsonRecord | null;
  try {
    record = asRecord(JSON.parse(result));
  } catch {
    return null;
  }
  if (
    !record ||
    record.blocked !== true ||
    record.readiness_status !== REQUIRE_SERVICE_BLOCKED_STATUS ||
    typeof record.reason_code !== "string" ||
    !record.reason_code.trim() ||
    typeof record.safe_message !== "string" ||
    !record.safe_message.trim() ||
    typeof record.service_slug !== "string"
  ) {
    return null;
  }
  const serviceSlug = record.service_slug.trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9._-]{0,127}$/.test(serviceSlug)) return null;
  const serviceLabel = humanizeAssistantServiceSlug(serviceSlug);
  const reasonCode: ChatAuthorizationReasonCode =
    record.reason_code === "NYXID_UNAUTHORIZED"
      ? "NYXID_UNAUTHORIZED"
      : "NYXID_SERVICE_NOT_CONNECTED";
  return {
    reasonCode,
    safeMessage:
      reasonCode === "NYXID_UNAUTHORIZED"
        ? `Reconnect ${serviceLabel} to continue.`
        : `Connect ${serviceLabel} to continue.`,
    serviceLabel,
    serviceSlug,
  };
}

export function authorizationBlockerToConnectCard(
  blocker: ChatAuthorizationBlocker,
): ConnectCardContentBlock {
  const action =
    blocker.reasonCode === "NYXID_UNAUTHORIZED" ? "Reconnect" : "Connect";
  return {
    type: "connect_card",
    block_id: `authorization:${blocker.serviceSlug}`,
    catalog_slug: blocker.serviceSlug || "custom",
    service_name: blocker.serviceLabel,
    icon_url: "",
    subtitle: "Required by this request",
    auth_kind: "api_key",
    requested_scopes: [],
    key_id: null,
    granted_scopes: null,
    device_user_code: null,
    device_verification_url: null,
    state: "needs_connection",
    error_message: null,
    steps: [
      {
        title: `${action} ${blocker.serviceLabel}`,
        body: blocker.safeMessage,
        done: false,
      },
    ],
    footer: "Brokered by NyxID - configure in AI Services, then ask again",
    reason_code: blocker.reasonCode,
  };
}
