import { translateApp, appLocale } from "../../i18n/appTranslation";
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
    ? translateApp("Reminder")
    : row.delivery.type === "agent"
      ? translateApp("Device agent")
      : translateApp("HTTPS webhook");
}

export function automationPauseReason(reason: string | null | undefined) {
  switch (reason) {
    case "target_unavailable":
      return translateApp(
        "The target agent is unavailable. Review Agent access before resuming.",
      );
    case "assistant_disabled":
      return translateApp(
        "NyxBot is disabled. Review your NyxBot settings before resuming.",
      );
    case "owner_unavailable":
      return translateApp("The owner account is unavailable.");
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

/**
 * A schedule described in the reader's local time. Calendar schedules run on
 * their own zone's clock, so the time (and, across midnight, the days) are
 * converted through a real occurrence: the next run, else today.
 */
export function automationScheduleSummary(row: Automation, zone: string): string {
  const schedule = row.schedule;
  if (!schedule || (row.source ?? "webhook") === "webhook")
    return translateApp("Webhook event");
  if (schedule.kind === "every")
    return (schedule.amount === 1 ? translateApp("Every {{amount}} {{unit}}", { amount: schedule.amount, unit: schedule.unit.replace(/s$/, "") }) : translateApp("Every {{amount}} {{unit}}", { amount: schedule.amount, unit: schedule.unit}));
  if (schedule.kind !== "cron") return translateApp("One time");
  const parsed = parseCron(schedule.expression);
  if (!parsed) return translateApp("Custom calendar schedule");
  const local = toLocalClock(
    parsed.hour,
    parsed.minute,
    schedule.timezone,
    zone,
    row.next_run_at ?? row.next_runs?.[0],
  );
  const at = clock(local.hour, local.minute);
  if (parsed.kind === "monthly") {
    // A day-of-month cannot move across midnight without changing meaning.
    if (local.shift !== 0)
      return translateApp(
        "Monthly on day {{day}} at {{clock}} {{timezone}} time",
        {
          day: parsed.day,
          clock: clock(parsed.hour, parsed.minute),
          timezone: schedule.timezone,
        },
      );
    return translateApp("Monthly on day {{day}} at {{at}}", {
      day: parsed.day,
      at: at,
    });
  }
  const days = new Set(
    [...parsed.days].map((day) => (((day + local.shift) % 7) + 7) % 7),
  );
  return translateApp("{{days}} at {{at}}", {
    days: describeDays(days),
    at: at,
  });
}

type ParsedCron =
  | { kind: "days"; hour: number; minute: number; days: Set<number> }
  | { kind: "monthly"; hour: number; minute: number; day: number };

/** Only calendar patterns we can explain exactly; never guess a run time. */
function parseCron(expression: string): ParsedCron | null {
  const fields = expression.trim().split(/\s+/);
  if (fields.length !== 5) return null;
  const [minute, hour, day, month, weekday] = fields;
  if (!/^\d{1,2}$/.test(minute!) || !/^\d{1,2}$/.test(hour!)) return null;
  const h = Number(hour),
    m = Number(minute);
  if (h > 23 || m > 59 || month !== "*") return null;
  if (weekday === "*" && /^\d{1,2}$/.test(day!) && Number(day) >= 1 && Number(day) <= 31)
    return { kind: "monthly", hour: h, minute: m, day: Number(day) };
  if (day !== "*") return null;
  const all = [0, 1, 2, 3, 4, 5, 6];
  const days =
    weekday === "*"
      ? all
      : weekday === "1-5"
        ? [1, 2, 3, 4, 5]
        : weekday === "0,6" || weekday === "6,0" || weekday === "6,7"
          ? [0, 6]
          : /^[0-7]$/.test(weekday!)
            ? [Number(weekday) % 7]
            : null;
  return days ? { kind: "days", hour: h, minute: m, days: new Set(days) } : null;
}

function describeDays(days: Set<number>) {
  const list = [...days].sort((a, b) => a - b);
  if (list.length === 7) return translateApp("Every day");
  if (list.join() === "1,2,3,4,5") return translateApp("Weekdays");
  if (list.join() === "0,6") return translateApp("Weekends");
  const dayPlural = [
    translateApp("Sundays"),
    translateApp("Mondays"),
    translateApp("Tuesdays"),
    translateApp("Wednesdays"),
    translateApp("Thursdays"),
    translateApp("Fridays"),
    translateApp("Saturdays"),
  ];
  const dayShort = [
    translateApp("Sun"),
    translateApp("Mon"),
    translateApp("Tue"),
    translateApp("Wed"),
    translateApp("Thu"),
    translateApp("Fri"),
    translateApp("Sat"),
  ];

  if (list.length === 1) return dayPlural[list[0]!]!;
  // A run that may wrap past Saturday (e.g. Tue–Sat, Sat–Wed).
  const start = list.find((day) => !days.has((day + 6) % 7));
  if (start !== undefined) {
    let end = start;
    while (days.has((end + 1) % 7) && (end + 1) % 7 !== start) end = (end + 1) % 7;
    if ((((end - start) % 7) + 7) % 7 === list.length - 1)
      return `${dayShort[start]}–${dayShort[end]}`;
  }
  return list.map((day) => dayShort[day]).join(", ");
}

function clock(hour: number, minute: number) {
  return `${hour % 12 || 12}:${String(minute).padStart(2, "0")} ${hour < 12 ? "AM" : "PM"}`;
}

function wall(instant: number, zone: string) {
  const parts = Object.fromEntries(
    new Intl.DateTimeFormat(appLocale(), {
      timeZone: zone,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
    })
      .formatToParts(new Date(instant))
      .map((part) => [part.type, part.value]),
  );
  return {
    date: Date.UTC(Number(parts.year), Number(parts.month) - 1, Number(parts.day)),
    hour: Number(parts.hour) % 24,
    minute: Number(parts.minute),
  };
}

/** The schedule zone's hour:minute on the reference day, read on the local clock. */
function toLocalClock(
  hour: number,
  minute: number,
  scheduleZone: string,
  localZone: string,
  reference?: string | null,
) {
  if (scheduleZone === localZone) return { hour, minute, shift: 0 };
  try {
    const day = wall(reference ? Date.parse(reference) : Date.now(), scheduleZone).date;
    const target = day + (hour * 60 + minute) * 60_000;
    // Find the instant whose wall clock in the schedule zone is the target.
    let instant = target;
    for (let i = 0; i < 2; i += 1) {
      const seen = wall(instant, scheduleZone);
      instant += target - (seen.date + (seen.hour * 60 + seen.minute) * 60_000);
    }
    const local = wall(instant, localZone);
    return {
      hour: local.hour,
      minute: local.minute,
      shift: Math.round((local.date - day) / 86_400_000),
    };
  } catch {
    return { hour, minute, shift: 0 };
  }
}

/** Status and run outcomes read as Title Case values ("Paused" for disabled). */
export function automationStatusLabel(value: string) {
  const text = value === "disabled" ? translateApp("Paused") : value.replaceAll("_", " ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

export function automationSourceLabel(row: Automation) {
  return (row.source ?? "webhook") === "schedule" ? translateApp("Scheduled")
    : translateApp("Webhook");
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
    automationScheduleSummary(row, zone),
    row.source === "schedule"
      ? row.status === "disabled"
        ? translateApp("Paused · no new scheduled runs")
        : translateApp("Next: {{date}}", {
            date: formatAutomationTime(row.next_run_at, zone),
          })
      : undefined,
    automationPauseReason(row.pause_reason),
  ]
    .filter(Boolean)
    .join("\n");
}

export function automationRunDuration(run: AutomationRun) {
  return run.duration_ms == null
    ? undefined
    : translateApp("{{seconds}} seconds", {
        seconds: (run.duration_ms / 1000).toFixed(1),
      });
}

/** The detail line of a run's history row. */
export function automationRunDetail(run: AutomationRun) {
  return [
    automationReason(run.reason),
    run.missed ? translateApp("Missed {{count}} occurrences", {
          count: run.missed.count,
        })
      : undefined,
    automationRunDuration(run),
  ]
    .filter(Boolean)
    .join(" · ");
}
