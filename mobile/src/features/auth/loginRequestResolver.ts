import { classifyLoginError, LoginRequestError } from "./loginRequestErrors";
export {
  classifyLoginError,
  LoginRequestError,
  loginErrorCopy,
  deviceLoginErrorCopy,
  isLoginDecisionUncertain,
  type LoginFailure,
} from "./loginRequestErrors";
import { mobileApi } from "../../lib/api/mobileApi";
import { agentKeyLoginApi } from "../../lib/api/agentKeyLoginApi";
import { getApiBaseUrl } from "../../lib/api/baseUrl";
import type { AuthDevicePreview } from "../../lib/api/authDeviceSchema";
import type { AgentKeyPreview } from "../../lib/api/agentKeyLoginSchema";
import { normalizeAuthDeviceUserCode } from "./deviceUserCode";

export type LoginRequest =
  | { kind: "device"; preview: AuthDevicePreview }
  | { kind: "agent-key"; preview: AgentKeyPreview };
export type LoginTerminal =
  | "approved"
  | "denied"
  | "expired"
  | "completed"
  | "unavailable";
export function loginServerHost(): string {
  return new URL(getApiBaseUrl()).host;
}
export function terminalFromError(
  error: unknown,
  uncertainDecision?: "approve" | "deny" | null,
): LoginTerminal | null {
  const kind = classifyLoginError(error);
  if (kind === "completed" && uncertainDecision === "approve") return "approved";
  return kind === "not_found"
    ? "unavailable"
    : kind === "expired" || kind === "completed" || kind === "denied"
      ? kind
      : null;
}
export function terminalFromPreview(
  preview: AuthDevicePreview,
  uncertainDecision?: "approve" | "deny" | null,
): LoginTerminal | null {
  if (
    uncertainDecision === "approve" &&
    (preview.status === "approved" || preview.status === "delivered")
  )
    return "approved";
  return preview.status === "pending"
    ? null
    : preview.status === "approved" || preview.status === "delivered"
      ? "completed"
      : preview.status;
}
type Previewers = {
  device: (code: string) => Promise<AuthDevicePreview>;
  "agent-key": (code: string) => Promise<AgentKeyPreview>;
};
let cooldownUntil = 0;
export function previewCooldownMs() {
  return Math.max(0, cooldownUntil - Date.now());
}
export function recordLoginPreviewFailure(error: unknown) {
  if (classifyLoginError(error) !== "rate_limited") return;
  const retry = (error as { retryAfterMs?: number } | null)?.retryAfterMs;
  const delay =
    typeof retry === "number" && Number.isFinite(retry)
      ? Math.min(300_000, Math.max(750, retry))
      : 5000;
  cooldownUntil = Math.max(cooldownUntil, Date.now() + delay);
}
export async function previewLoginRequest(
  code: string,
  kind: LoginRequest["kind"],
  schema: LoginRequest["kind"] = kind,
): Promise<AuthDevicePreview> {
  if (previewCooldownMs())
    throw new LoginRequestError("rate_limited", previewCooldownMs());
  try {
    return await (kind === "device" && schema === "device"
      ? mobileApi.previewAuthDevice(code)
      : agentKeyLoginApi.preview(code, kind));
  } catch (error) {
    recordLoginPreviewFailure(error);
    throw error;
  }
}
export async function resolveLoginRequest(
  code: string,
  order: LoginRequest["kind"],
  previews: Previewers = {
    device: mobileApi.previewAuthDevice,
    "agent-key": agentKeyLoginApi.preview,
  },
): Promise<LoginRequest> {
  const normalized = normalizeAuthDeviceUserCode(code);
  if (!normalized) throw new LoginRequestError("malformed");
  if (previewCooldownMs())
    throw new LoginRequestError("rate_limited", previewCooldownMs());
  const kinds = [order, order === "device" ? "agent-key" : "device"] as const;
  for (const kind of kinds) {
    try {
      if (kind === "device")
        return { kind, preview: await previews.device(normalized) };
      return { kind, preview: await previews["agent-key"](normalized) };
    } catch (error) {
      if (classifyLoginError(error) === "rate_limited") {
        recordLoginPreviewFailure(error);
        throw new LoginRequestError("rate_limited", previewCooldownMs());
      }
      const code = (error as { errorCode?: number } | null)?.errorCode;
      if (code !== (kind === "device" ? 11207 : 11907))
        throw new LoginRequestError(classifyLoginError(error));
    }
  }
  throw new LoginRequestError("not_found");
}

// Manual resolution publishes an exact, short-lived cached preview for the
// destination. Device links without one separately load a public preview through
// the shared resolver; neither path makes an approval decision.
let handoff: { code: string; result: LoginRequest; at: number } | null = null;
let handoffTimeout: ReturnType<typeof setTimeout> | undefined;
function clearHandoff() {
  handoff = null;
  clearTimeout(handoffTimeout);
  handoffTimeout = undefined;
}
export function handoffLoginRequest(code: string, result: LoginRequest) {
  clearHandoff();
  const value = {
    code: normalizeAuthDeviceUserCode(code) ?? "",
    result,
    at: Date.now(),
  };
  handoff = value;
  const release = () => {
    if (handoff === value) clearHandoff();
  };
  handoffTimeout = setTimeout(release, 10_000);
  return release;
}
export function takeLoginRequest(code: string, kind: LoginRequest["kind"]) {
  const value = handoff;
  if (
    !value ||
    value.code !== normalizeAuthDeviceUserCode(code) ||
    value.result.kind !== kind
  )
    return null;
  clearHandoff();
  return Date.now() - value.at <= 10_000 ? value : null;
}
