import { voiceStartDetailsSchema } from "@/schemas/assistant-voice";
import { ApiError } from "./api-client";

export type VoiceStartStep =
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
      return `${error.message} (${hint} · code ${error.errorCode})`;
    }
    const message =
      error.errorCode === 11300
        ? "Insufficient voice credits"
        : error.errorCode === 11307
          ? "Voice billing wallet is suspended"
          : error.errorCode === 11301 || error.errorCode === 11302
            ? "Voice billing is unavailable"
            : error.message;
    return `${message} (code ${error.errorCode})`;
  }
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
