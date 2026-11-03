import type { AssistantAgent } from "../agents/agentContracts";
import type {
  Automation,
  AutomationRun,
  AutomationRunsPage,
} from "./automationContracts";
import { formatAutomationTime } from "./automationTime";

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

export function automationScheduleSummary(row: Automation): string {
  const schedule = row.schedule;
  if (!schedule || (row.source ?? "webhook") === "webhook")
    return "Webhook event";
  if (schedule.kind === "cron")
    return `${cronSummary(schedule.expression)} (${schedule.timezone})`;
  if (schedule.kind === "every")
    return `Every ${schedule.amount} ${schedule.amount === 1 ? schedule.unit.replace(/s$/, "") : schedule.unit}`;
  return "One time";
}

/** Describe only calendar patterns we can explain exactly; never guess a run time. */
function cronSummary(expression: string) {
  const fields = expression.trim().split(/\s+/);
  if (fields.length !== 5) return "Custom calendar schedule";
  const [minute, hour, day, month, weekday] = fields;
  if (!/^\d{1,2}$/.test(minute!) || !/^\d{1,2}$/.test(hour!))
    return "Custom calendar schedule";
  const h = Number(hour),
    m = Number(minute);
  if (h > 23 || m > 59 || month !== "*") return "Custom calendar schedule";
  const at = `${h % 12 || 12}:${String(m).padStart(2, "0")} ${h < 12 ? "AM" : "PM"}`;
  if (
    weekday === "*" &&
    /^\d{1,2}$/.test(day!) &&
    Number(day) >= 1 &&
    Number(day) <= 31
  )
    return `Monthly on day ${Number(day)} at ${at}`;
  if (day !== "*") return "Custom calendar schedule";
  if (weekday === "*") return `Every day at ${at}`;
  if (weekday === "1-5") return `Weekdays at ${at}`;
  if (weekday === "0,6" || weekday === "6,0" || weekday === "6,7")
    return `Weekends at ${at}`;
  if (/^[0-7]$/.test(weekday!))
    return `${["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"][Number(weekday) % 7]} at ${at}`;
  return "Custom calendar schedule";
}

/** Status and run outcomes read as Title Case values ("Paused" for disabled). */
export function automationStatusLabel(value: string) {
  const text = value === "disabled" ? "Paused" : value.replaceAll("_", " ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function automationSourceLabel(row: Automation) {
  return (row.source ?? "webhook") === "schedule" ? "Scheduled" : "Webhook";
}

/** Server reason codes read as words ("confirmation_required" -> "confirmation required"). */
export function automationReason(reason: string | null | undefined) {
  return reason ? reason.replaceAll("_", " ") : undefined;
}

/** The detail line of an automation's list row: target, timing and any pause. */
export function automationListDetail(
  row: Automation,
  target: string,
  zone: string,
) {
  return [
    `${target} · ${automationSourceLabel(row)}`,
    automationScheduleSummary(row),
    row.source === "schedule"
      ? row.status === "disabled"
        ? "Paused · no new scheduled runs"
        : `Next: ${formatAutomationTime(row.next_run_at, zone)}`
      : undefined,
    automationPauseReason(row.pause_reason),
  ]
    .filter(Boolean)
    .join("\n");
}

export function automationRunDuration(run: AutomationRun) {
  return run.duration_ms == null
    ? undefined
    : `${(run.duration_ms / 1000).toFixed(1)} seconds`;
}

/** The detail line of a run's history row. */
export function automationRunDetail(run: AutomationRun) {
  return [
    automationReason(run.reason),
    run.missed ? `Missed ${run.missed.count} occurrences` : undefined,
    automationRunDuration(run),
  ]
    .filter(Boolean)
    .join(" · ");
}
