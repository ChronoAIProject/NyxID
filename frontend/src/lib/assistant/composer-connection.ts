import { classifyConnection } from "@/lib/service-routing-preview";
import type { KeyInfo } from "@/types/keys";

/** Execution hints apply to this message, never to durable agent grants. */
export function composerConnectionUnavailableReason(
  connection: KeyInfo,
  now = Date.now(),
): string | null {
  // The backend checks alternate node bindings before reporting this reason.
  // A node's status alone cannot establish that execution is unavailable.
  if (connection.execution_unavailable_reason) {
    return connection.execution_unavailable_reason;
  }
  const candidate = classifyConnection(connection, [], now);
  return candidate.state === "unavailable" ? candidate.reason : null;
}
