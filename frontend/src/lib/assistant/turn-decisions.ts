import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

/** Settled decisions only: pending cards stay in the transcript as cards. */
type Decision = NyxAgentAcknowledgement;

/** Allowed without asking because the agent runs with Full access. */
export function isAutomatic(row: Decision): boolean {
  return row.decided_by === "access_mode";
}

/** Who decided and how, in a couple of words. */
export function decisionLabel(row: Decision): string {
  if (isAutomatic(row)) return "Full access";
  const by = row.decided_by === "orchestrator" ? "NyxBot" : row.decided_by === "user" ? "You" : undefined;
  switch (row.status) {
    case "allowed":
      return by ? `${by} allowed` : "Allowed";
    case "denied":
      return by ? `${by} denied` : "Denied";
    case "used":
      return "Used";
    default:
      return "Expired";
  }
}

const RECORD_ID = /\s*\([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\)/gi;

/** The receipt's summary without the record IDs the server embeds in it. */
export function decisionText(row: Decision): string {
  return row.summary.replace(RECORD_ID, "").replace(/\s+/g, " ").trim();
}

export interface DecisionCounts {
  readonly automatic: number;
  readonly allowed: number;
  readonly denied: number;
  readonly expired: number;
}

export function countDecisions(rows: readonly Decision[]): DecisionCounts {
  let automatic = 0;
  let allowed = 0;
  let denied = 0;
  let expired = 0;
  for (const row of rows) {
    if (isAutomatic(row)) automatic += 1;
    else if (row.status === "denied") denied += 1;
    else if (row.status === "expired") expired += 1;
    else allowed += 1;
  }
  return { automatic, allowed, denied, expired };
}

/** Header fragments for a turn's decisions, most important first. */
export function decisionSummary(counts: DecisionCounts): string[] {
  return [
    counts.denied ? `${String(counts.denied)} denied` : "",
    counts.allowed ? `${String(counts.allowed)} allowed` : "",
    counts.automatic ? `${String(counts.automatic)} allowed by Full access` : "",
    counts.expired ? `${String(counts.expired)} expired` : "",
  ].filter(Boolean);
}

/** Identical decisions (same verdict, same action) collapse into one line. */
export function groupDecisions(
  rows: readonly Decision[],
): { readonly row: Decision; readonly count: number }[] {
  const groups = new Map<string, { row: Decision; count: number }>();
  for (const row of rows) {
    const key = `${decisionLabel(row)}\u0000${decisionText(row)}\u0000${row.reason ?? ""}`;
    const group = groups.get(key);
    if (group) group.count += 1;
    else groups.set(key, { row, count: 1 });
  }
  return [...groups.values()];
}

/** The full receipt, for a tooltip: the verdict, what bypassed which check, and the action. */
export function decisionTitle(row: Decision): string {
  const bypass = isAutomatic(row)
    ? (
        {
          webhook: "automation confirmation",
          policy: "policy confirmation",
          approval: "service approval",
          access_mode: "",
        } as const
      )[row.confirmation?.reason ?? "access_mode"]
    : "";
  return [
    isAutomatic(row) ? "Allowed automatically by Full access" : decisionLabel(row),
    bypass ? `bypassed ${bypass}` : "",
    row.summary,
    row.reason?.trim() ?? "",
  ]
    .filter(Boolean)
    .join(" · ");
}
