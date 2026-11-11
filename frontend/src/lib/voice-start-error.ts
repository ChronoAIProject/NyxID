import { voiceStartDetailsSchema } from "@/schemas/assistant-voice";
import { ApiError } from "./api-client";
import { isInsufficientCreditsHttp } from "./credits-denial";

export type VoiceStartStep =
  | "settings"
  | "microphone"
  | "webrtc"
  | "server"
  | "response"
  | "transport";

export function voiceStartError(error: unknown, step: VoiceStartStep): string {
  if (error instanceof ApiError) {
    if (
      error.status === 409 &&
      /^(Conflict: )?End the current call first$/.test(error.message)
    )
      return "End the current call first";
    const recovery = isInsufficientCreditsHttp(
      error.status,
      error.errorResponse,
    )
      ? " Check your available credits, then retry voice."
      : "";
    const details = voiceStartDetailsSchema.safeParse(
      error.errorResponse.details,
    );
    if (details.success) {
      const d = details.data;
      const provider =
        d.provider === "openai"
          ? "OpenAI"
          : d.provider === "xai"
            ? "xAI"
            : undefined;
      const category = d.provider_code ?? d.provider_type;
      const hint = provider
        ? `${provider}${d.provider_status ? ` ${d.provider_status}` : ""}${category ? `: ${category}` : ""}${d.provider_param ? ` · ${d.provider_param}` : ""}`
        : d.stage;
      return `${error.message} (${hint} · code ${error.errorCode})${recovery}`;
    }
    const billingCode = error.status === 402 ? error.errorCode : undefined;
    const message =
      billingCode === 11300
        ? "Insufficient voice credits"
        : billingCode === 11307
          ? "Voice billing wallet is suspended"
          : billingCode === 11301 || billingCode === 11302
            ? "Voice billing is unavailable"
            : error.message;
    return `${message} (code ${error.errorCode})${recovery}`;
  }
  if (step === "settings")
    return "Voice preferences could not be saved. Please try again.";
  if (step === "microphone") {
    const name =
      error instanceof Error || error instanceof DOMException ? error.name : "";
    if (name === "NotAllowedError") return "Microphone permission was denied";
    if (name === "NotFoundError") return "No microphone found";
    return "Microphone could not be opened";
  }
  if (step === "webrtc") return "WebRTC negotiation failed";
  if (step === "response")
    return "Voice server returned an invalid session answer";
  if (step === "transport") return "Voice control connection could not open";
  return "Voice server could not be reached";
}
