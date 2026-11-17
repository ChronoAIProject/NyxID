import type { NyxAgentSettingsForm } from "@/schemas/assistant-nyxagent";
import type { NyxBotSettingsSection } from "./nyxbot-settings-sections";
import {
  DEFAULT_SCHEDULE_MINIMUM_MINUTES,
  SCHEDULE_MINIMUM_MINUTES_LIMIT,
  TRIGGER_RUNS_PER_HOUR_LIMIT,
  TRIGGER_RUNS_PER_DAY_LIMIT,
} from "@/lib/automation-limits";

/** Shared by the rendered fields, change review and workspace search. */
export const SETTINGS_LABELS = {
  max_auto_continuations: "Automatic task continuations",
  max_live_subagents: "Live specialists",
  max_concurrent_subagent_turns: "Specialists working at once",
  max_group_handoffs: "Group hand-offs per message",
  max_group_handoffs_per_hour: "Group hand-offs per hour",
} as const;

export const SETTINGS_PAGE_FIELDS: Partial<
  Record<NyxBotSettingsSection, readonly (keyof NyxAgentSettingsForm)[]>
> = {
  permissions: ["access_mode", "specialist_access_requests"],
  tasks: ["max_auto_continuations", "max_live_subagents", "max_concurrent_subagent_turns", "max_group_handoffs", "max_group_handoffs_per_hour"],
};

export const SPECIALIST_ACCESS_LABELS = {
  denied: "Deny requests",
  routed_to_nyxbot: "Route to NyxBot",
} as const;

export const AUTOMATION_TIMEZONE_LABEL = "Timezone";
export const AUTOMATION_BUDGET_ROWS = [
  ["schedule_minimum_minutes", "Minimum interval (minutes)", DEFAULT_SCHEDULE_MINIMUM_MINUTES, SCHEDULE_MINIMUM_MINUTES_LIMIT],
  ["trigger_runs_per_hour", "Runs per hour", 1, TRIGGER_RUNS_PER_HOUR_LIMIT],
  ["trigger_runs_per_day", "Runs per day", 1, TRIGGER_RUNS_PER_DAY_LIMIT],
] as const;
