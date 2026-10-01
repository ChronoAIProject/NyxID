import { ApiError } from "@/lib/api-client";
import type { PoolCandidate } from "@/schemas/pools";

export const strategyLabels = {
  priority: "Automatic fallback",
  round_robin: "Round robin",
  weighted: "Weighted balancing",
};
const reasonLabels: Record<string, string> = {
  unavailable: "Connection unavailable",
  inactive: "Service disabled",
  disabled: "Member disabled",
  cooldown: "Cooling down",
  incompatible_protocol: "Protocol is incompatible with this pool",
  compatibility_declaration_required:
    "Confirm compatibility for every affected member",
  inference_protocol_required:
    "Catalog inference metadata or a supported chat operation is required",
  operation_unsupported: "Operation is not permitted",
  node_upgrade_required: "Upgrade the node for HTTP cancellation support",
  node_offline: "Node is offline",
  unsupported_transport: "Transport is not supported",
};
export function bindingLabel(binding: string) {
  return (
    (
      {
        platform: "Platform access",
        user: "Your credentials",
        none: "No credentials needed",
        unavailable: "Unavailable",
      } as Record<string, string>
    )[binding] ?? binding
  );
}
export function protocolLabel(protocol: string) {
  return (
    (
      {
        openai_completions: "OpenAI Chat",
        openai_responses: "OpenAI Responses",
        anthropic_messages: "Anthropic Messages",
      } as Record<string, string>
    )[protocol] ?? protocol
  );
}
export function reason(candidate: PoolCandidate) {
  return candidate.reason
    ? (reasonLabels[candidate.reason] ?? candidate.reason.replaceAll("_", " "))
    : "Eligible";
}
export function message(error: unknown) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : "Unable to save pool";
}
