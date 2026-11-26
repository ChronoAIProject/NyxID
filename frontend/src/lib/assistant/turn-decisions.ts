import { translateUi } from "@/i18n/use-ui-translation";
import type { ActivityDecision } from "./activity-decision";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

/** Settled decisions only: pending cards stay in the transcript as cards. */
type Decision = NyxAgentAcknowledgement;

/** Allowed without asking because the agent runs with Full access. */
export function isAutomatic(row: Decision): boolean {
  return row.decided_by === "access_mode";
}

/** Who decided and how, in a couple of words. */
export function decisionLabel(row: Decision, t = translateUi): string {
  if (isAutomatic(row)) return t("Full access");
  switch (row.status) {
    case "allowed":
      return row.decided_by === "orchestrator" ? t("NyxBot allowed") : row.decided_by === "user" ? t("You allowed") : t("Allowed");
    case "denied":
      return row.decided_by === "orchestrator" ? t("NyxBot denied") : row.decided_by === "user" ? t("You denied") : t("Denied");
    case "used":
      return t("Used");
    default:
      return t("Expired");
  }
}

const RECORD_ID = /\s*\([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\)/gi;

/** The receipt's summary without the record IDs the server embeds in it. */
export function decisionText(row: Decision): string {
  return row.summary.replace(RECORD_ID, "").replace(/\s+/g, " ").trim();
}

export interface DecisionCounts {
  readonly waiting?: number;
  readonly cancelled?: number;
  readonly unavailable?: number;
  readonly automatic: number;
  readonly allowed: number;
  readonly denied: number;
  readonly expired: number;
}

export function countDecisions(rows: readonly Decision[], links: readonly ActivityDecision[] = []): DecisionCounts {
  let automatic = 0;
  let allowed = 0;
  let denied = 0;
  let expired = 0;
  const linked = new Map(links.map((link) => [link.id, link]));
  for (const row of rows) {
    if (linked.has(row.id)) continue;
    if (isAutomatic(row)) automatic += 1;
    else if (row.status === "denied") denied += 1;
    else if (row.status === "expired") expired += 1;
    else allowed += 1;
  }
  if (!linked.size) return { automatic, allowed, denied, expired };
  let waiting = 0, cancelled = 0, unavailable = 0;
  for (const link of linked.values()) {
    switch (link.state) {
      case "waiting": waiting += 1; break;
      case "allowed": case "completed": case "failed": allowed += 1; break;
      case "denied": denied += 1; break;
      case "expired": expired += 1; break;
      case "cancelled": cancelled += 1; break;
      default: unavailable += 1;
    }
  }
  return { automatic, allowed, denied, expired, waiting, cancelled, unavailable };
}

/** Header fragments for a turn's decisions, most important first. */
export function decisionSummary(counts: DecisionCounts, t = translateUi) {
  const labels = {
    waiting: "{{count}} waiting",
    cancelled: "{{count}} cancelled",
    unavailable: "{{count}} outcome unavailable",
    denied: "{{count}} denied",
    allowed: "{{count}} allowed",
    automatic: "{{count}} allowed by Full access",
    expired: "{{count}} expired",
  } satisfies Record<keyof DecisionCounts, string>;
  return (Object.keys(labels) as (keyof DecisionCounts)[]).flatMap(kind => {
    const count = counts[kind];
    return count ? [{ label: t(labels[kind], { count }), warning: kind === "denied" }] : [];
  });
}

/** Identical decisions (same verdict, same action) collapse into one line. */
export function groupDecisions(
  rows: readonly Decision[],
  selectedId?: string,
): { readonly row: Decision; readonly count: number }[] {
  const groups = new Map<string, { row: Decision; count: number }>();
  for (const row of rows) {
    const key = `${decisionLabel(row)}\u0000${decisionText(row)}\u0000${row.reason ?? ""}`;
    const group = groups.get(key);
    if (group) {
      group.count += 1;
      if (row.id === selectedId) group.row = row;
    }
    else groups.set(key, { row, count: 1 });
  }
  return [...groups.values()];
}

/** The full receipt, for a tooltip: the verdict, what bypassed which check, and the action. */
export function decisionTitle(row: Decision, t = translateUi): string {
  const bypass = isAutomatic(row)
    ? (
        {
          webhook: "automation confirmation",
          policy: "policy confirmation",
          org_connection: "organization instruction",
          delegation: "delegation confirmation",
          approval: "service approval",
          access_mode: "",
        } as const
      )[row.confirmation?.reason ?? "access_mode"]
    : "";
  return [
    isAutomatic(row) ? t("Allowed automatically by Full access") : decisionLabel(row, t),
    bypass ? t("bypassed {{reason}}", { reason: t(bypass) }) : "",
    row.summary,
    row.reason?.trim() ?? "",
  ]
    .filter(Boolean)
    .join(" · ");
}
