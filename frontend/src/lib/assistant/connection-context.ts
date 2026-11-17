import type { KeyInfo } from "@/types/keys";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";

/** The same connection inventory backs the Access summary and its list. */
export function connectionsForAgent(agent: AssistantAgent, inventory: readonly KeyInfo[]) {
  return agent.kind === "nyxbot" ? inventory : inventory.filter(
    key => agent.services.includes(key.id) || agent.services.includes(key.slug),
  );
}

export function connectionOwnerLabel(connection: KeyInfo): string {
  return connection.credential_source?.type === "org"
    ? connection.credential_source.org_name
    : "Personal";
}

export function connectionAvailable(connection: KeyInfo): boolean {
  return (
    connection.is_active &&
    !(
      connection.credential_source?.type === "org" &&
      !connection.credential_source.allowed
    )
  );
}
