import { classifyConnection } from "@/lib/service-routing-preview";
import type { KeyInfo } from "@/types/keys";
import { translateUi } from "@/i18n/use-ui-translation";

export type ComposerConnectionReason =
  | { readonly kind: "classifier"; readonly reason: string }
  | { readonly kind: "node"; readonly name: string; readonly status: string }
  | { readonly kind: "backend"; readonly text: string };

/** Execution hints apply to this message, never to durable agent grants. */
export function composerConnectionUnavailableReason(
  connection: KeyInfo,
  now = Date.now(),
): ComposerConnectionReason | null {
  const candidate = classifyConnection(connection, [], now);
  if (candidate.state === "unavailable") {
    return { kind: "classifier", reason: candidate.reason };
  }
  // The backend checks alternate node bindings before reporting this reason.
  // A node's status alone cannot establish that execution is unavailable.
  if (connection.execution_unavailable_reason) {
    const text = connection.execution_unavailable_reason;
    const node = /^node (.+) (\S+)$/s.exec(text);
    return node
      ? { kind: "node", name: node[1]!, status: node[2]! }
      : { kind: "backend", text };
  }
  return null;
}

/** Only fixed copy is translated; names and unknown backend text stay values. */
export function formatComposerConnectionReason(
  reason: ComposerConnectionReason,
): string {
  if (reason.kind === "backend") {
    return reason.text.trim() ? reason.text : translateUi("Connection unavailable");
  }
  if (reason.kind === "classifier") return translateUi(reason.reason);
  const { name, status } = reason;
  switch (status) {
    case "offline":
      return translateUi("Node {{name}} is offline", { name });
    case "draining":
      return translateUi("Node {{name}} is draining", { name });
    case "disabled":
      return translateUi("Node {{name}} is disabled", { name });
    default:
      return translateUi("Node {{name}} is unavailable", { name });
  }
}

export function composerConnectionOwnerLabel(connection: KeyInfo): string {
  return connection.credential_source?.type === "org"
    ? connection.credential_source.org_name
    : translateUi("Personal");
}
