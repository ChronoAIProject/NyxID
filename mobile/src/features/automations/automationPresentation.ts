import type { AssistantAgent } from "../agents/agentContracts";
import type {
  Automation,
  AutomationRun,
  AutomationRunsPage,
} from "./automationContracts";

export function ownedAutomationAgents(
  agents: AssistantAgent[],
  userId?: string,
) {
  if (!userId) return [];
  return agents.filter(
    (agent) =>
      agent.status !== "destroyed" &&
      agent.owner_kind !== "org" &&
      agent.can_use !== false &&
      (agent.owner_id
        ? agent.owner_id === userId
        : agent.can_maintain !== false),
  );
}

export function automationTarget(
  row: Automation,
  agents: AssistantAgent[] = [],
) {
  if (row.delivery.type === "assistant") {
    const id = row.delivery.agent_id;
    const agent = agents.find((item) => item.id === id);
    return agent?.display_name || agent?.name || id;
  }
  return row.delivery.type === "notification"
    ? "Reminder"
    : row.delivery.type === "agent"
      ? "Device agent"
      : "HTTPS webhook";
}

export function automationPauseReason(reason: string | null | undefined) {
  switch (reason) {
    case "target_unavailable":
      return "The target agent is unavailable. Review Agent access before resuming.";
    case "assistant_disabled":
      return "NyxBot is disabled. Review your NyxBot settings before resuming.";
    case "owner_unavailable":
      return "The owner account is unavailable.";
    default:
      return reason?.replaceAll("_", " ");
  }
}

export function automationRunNotice(run: AutomationRun) {
  switch (run.outcome) {
    case "pending":
      return "Run queued. Check run history for its progress and result.";
    case "started":
      return "Run started. Check run history for its result.";
    case "waiting":
      return "Run is waiting. Open its thread to review any required confirmation.";
    case "completed":
      return "Run completed. Open run history to view its thread.";
    case "failed":
      return "Run failed. Check run history for the reason.";
    case "skipped":
      return "Run skipped. Check run history for the reason.";
  }
}

export function uniqueAutomationRuns(pages: AutomationRunsPage[]) {
  const seen = new Set<string>();
  return pages
    .flatMap((page) => page.runs)
    .filter((run) => {
      if (seen.has(run.id)) return false;
      seen.add(run.id);
      return true;
    });
}
