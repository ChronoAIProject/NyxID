import { translateApp } from "../../../i18n/appTranslation";
import { ApiError } from "../../../lib/api/ApiError";
import { voiceStartDetailsSchema } from "./voiceContracts";

export type VoiceStartStep =
  | "microphone"
  | "webrtc"
  | "server"
  | "response"
  | "transport";

export class NativeVoiceRuntimeUnavailableError extends Error {}

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
      message: translateApp(
        "There are not enough available credits for the initial voice reservation. Open Billing to review your balance and applicable benefits, or choose another eligible voice source.",
      ),
    };
  if (error.errorCode === 11307)
    return {
      kind: "wallet_suspended",
      message: translateApp(
        "Your billing wallet is suspended. Open Billing to review your wallet status before starting voice again.",
      ),
    };
  if (error.errorCode === 11301 || error.errorCode === 11302)
    return {
      kind: "billing_unavailable",
      message: translateApp(
        "Voice billing is temporarily unavailable. Try again later, or choose another eligible voice source.",
      ),
    };
  return undefined;
}

type StartStage = ReturnType<typeof voiceStartDetailsSchema.parse>["stage"];
const stageMessages: Record<StartStage, string> = {
  get flag() {
    return translateApp("Voice is not enabled for this account");
  },
  get thread() {
    return translateApp("Voice is unavailable for this conversation");
  },
  get origin() {
    return translateApp("Voice origin was refused");
  },
  get credential() {
    return translateApp("Voice credential is unavailable or not authorized");
  },
  get billing_reservation() {
    return translateApp("Voice billing could not be reserved");
  },
  get provider_create() {
    return translateApp("Voice provider could not create the session");
  },
  get provider_answer() {
    return translateApp("Voice provider returned an invalid session answer");
  },
  get transport() {
    return translateApp("Voice transport could not connect");
  },
};

// The contract already bounds identifiers and excludes prose and URLs. Apply
// the backend's additional credential-prefix rule before showing any metadata.
function diagnosticIdentifier(value?: string) {
  return value && !/^(?:sk[-_]|nyx_|bearer)/i.test(value) ? value : undefined;
}

function apiMessage(error: ApiError) {
  if (error.statusCode === 409)
    return translateApp(
      "Voice state changed before the call could start. Check call status and try again.",
    );
  if (error.errorCode === 11300) return translateApp("Insufficient voice credits");
  if (error.errorCode === 11307) return translateApp("Voice billing wallet is suspended");
  if (error.errorCode === 11301 || error.errorCode === 11302)
    return translateApp("Voice billing is unavailable");
  if (error.errorCode === 12500) return translateApp("Voice is busy. Try again shortly");
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
          ? translateApp("Voice provider rejected the session")
          : stageMessages[d.stage]);
      return `${message} (${hint}${code ? ` · ${code}` : ""})`;
    }
    const message =
      known ??
      (error.statusCode === 401
        ? translateApp(
            "Your session has expired. Sign in again before starting voice",
          )
        : error.statusCode === 403
          ? translateApp(
              "Voice access was refused. Check your account and provider connection",
            )
          : error.statusCode === 429
            ? translateApp(
                "Too many voice requests. Wait a moment and try again",
              )
            : error.statusCode === 400 || error.statusCode === 422
              ? translateApp(
                  "Voice settings were rejected. Check your source and voice",
                )
              : translateApp("Voice server could not start the call"));
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
    if (error instanceof NativeVoiceRuntimeUnavailableError)
      return translateApp("Voice audio is not supported by this app runtime.");
    const name = error instanceof Error ? error.name : "";
    if (name === "NotAllowedError") return translateApp("Microphone permission was denied");
    if (name === "NotFoundError") return translateApp("No microphone found");
    return translateApp("Microphone could not be opened");
  }
  if (step === "webrtc") return translateApp("WebRTC negotiation failed");
  if (step === "response")
    return translateApp("Voice server returned an invalid session answer");
  if (step === "transport") return translateApp("Voice control connection could not open");
  return translateApp(
    "Voice server could not be reached. Check the active call before trying again.",
  );
}
