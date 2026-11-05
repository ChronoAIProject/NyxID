import { ApiError } from "../../../lib/api/ApiError";
import { voiceStartDetailsSchema } from "./voiceContracts";

export type VoiceStartStep =
  | "microphone"
  | "webrtc"
  | "server"
  | "response"
  | "transport";

export type VoiceStartRecovery = {
  kind: "credits" | "wallet_suspended" | "billing_unavailable";
  message: string;
};

/** Billing codes overlap other APIs, so recovery also requires the voice stage. */
export function voiceStartRecovery(
  error: unknown,
  step: VoiceStartStep,
): VoiceStartRecovery | undefined {
  if (!(error instanceof ApiError) || step !== "server") return undefined;
  const details = voiceStartDetailsSchema.safeParse(error.details);
  if (!details.success || details.data.stage !== "billing_reservation")
    return undefined;
  if (error.errorCode === 11300)
    return {
      kind: "credits",
      message:
        "There are not enough available credits for the initial voice reservation. Open Billing to review your balance and applicable benefits, or choose another eligible voice source.",
    };
  if (error.errorCode === 11307)
    return {
      kind: "wallet_suspended",
      message:
        "Your billing wallet is suspended. Open Billing to review your wallet status before starting voice again.",
    };
  if (error.errorCode === 11301 || error.errorCode === 11302)
    return {
      kind: "billing_unavailable",
      message:
        "Voice billing is temporarily unavailable. Try again later, or choose another eligible voice source.",
    };
  return undefined;
}

type StartStage = ReturnType<typeof voiceStartDetailsSchema.parse>["stage"];
const stageMessages: Record<StartStage, string> = {
  flag: "Voice is not enabled for this account",
  thread: "Voice is unavailable for this conversation",
  origin: "Voice origin was refused",
  credential: "Voice credential is unavailable or not authorized",
  billing_reservation: "Voice billing could not be reserved",
  provider_create: "Voice provider could not create the session",
  provider_answer: "Voice provider returned an invalid session answer",
  transport: "Voice transport could not connect",
};

// The contract already bounds identifiers and excludes prose and URLs. Apply
// the backend's additional credential-prefix rule before showing any metadata.
function diagnosticIdentifier(value?: string) {
  return value && !/^(?:sk[-_]|nyx_|bearer)/i.test(value) ? value : undefined;
}

function apiMessage(error: ApiError) {
  if (error.statusCode === 409)
    return "Another voice call is active. End it before starting a new call.";
  if (error.errorCode === 11300) return "Insufficient voice credits";
  if (error.errorCode === 11307) return "Voice billing wallet is suspended";
  if (error.errorCode === 11301 || error.errorCode === 11302)
    return "Voice billing is unavailable";
  if (error.errorCode === 12500) return "Voice is busy. Try again shortly";
  return undefined;
}

/**
 * Matches Web's startup diagnostics using mobile's ApiError fields. All display
 * copy is fixed; arbitrary API prose, provider bodies, SDP and device errors
 * never become UI text. The numeric code and validated machine identifiers are
 * enough to distinguish configuration, billing and provider failures.
 */
export function voiceStartError(error: unknown, step: VoiceStartStep): string {
  if (error instanceof ApiError) {
    const known = apiMessage(error);
    const code = Number.isSafeInteger(error.errorCode)
      ? `code ${error.errorCode}`
      : undefined;
    const details = voiceStartDetailsSchema.safeParse(error.details);
    if (details.success) {
      const d = details.data;
      const provider =
        d.provider === "openai"
          ? "OpenAI"
          : d.provider === "xai"
            ? "xAI"
            : null;
      const category =
        diagnosticIdentifier(d.provider_code) ??
        diagnosticIdentifier(d.provider_type);
      const parameter = diagnosticIdentifier(d.provider_param);
      const hint = provider
        ? `${provider}${d.provider_status ? ` ${d.provider_status}` : ""}${category ? `: ${category}` : ""}${parameter ? ` · ${parameter}` : ""}`
        : d.stage;
      const message =
        known ??
        (d.stage === "provider_create" && d.provider_status
          ? "Voice provider rejected the session"
          : stageMessages[d.stage]);
      return `${message} (${hint}${code ? ` · ${code}` : ""})`;
    }
    const message =
      known ??
      (error.statusCode === 401
        ? "Your session has expired. Sign in again before starting voice"
        : error.statusCode === 403
          ? "Voice access was refused. Check your account and provider connection"
          : error.statusCode === 429
            ? "Too many voice requests. Wait a moment and try again"
            : error.statusCode === 400 || error.statusCode === 422
              ? "Voice settings were rejected. Check your source and voice"
              : "Voice server could not start the call");
    const status =
      Number.isInteger(error.statusCode) &&
      error.statusCode >= 100 &&
      error.statusCode <= 599
        ? `HTTP ${error.statusCode}`
        : undefined;
    const hint = [status, code].filter(Boolean).join(" · ");
    return `${message}${hint ? ` (${hint})` : ""}`;
  }
  if (step === "microphone") {
    const name = error instanceof Error ? error.name : "";
    if (name === "NotAllowedError") return "Microphone permission was denied";
    if (name === "NotFoundError") return "No microphone found";
    return "Microphone could not be opened";
  }
  if (step === "webrtc") return "WebRTC negotiation failed";
  if (step === "response")
    return "Voice server returned an invalid session answer";
  if (step === "transport") return "Voice control connection could not open";
  return "Voice server could not be reached. Check the active call before trying again.";
}
