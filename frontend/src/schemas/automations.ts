import {
  DEFAULT_SCHEDULE_MINIMUM_MINUTES,
  SCHEDULE_MINIMUM_MINUTES_LIMIT,
  TRIGGER_RUNS_PER_HOUR_LIMIT,
  TRIGGER_RUNS_PER_DAY_LIMIT,
} from "@/lib/automation-limits";
import { automationInstant, localTime } from "@/lib/automation-time";
import { z } from "zod";
import type { CreateTriggerRequest, TriggerResponse } from "./triggers";

const instant = z.string().datetime({ offset: true });
const options = {
  start: instant.nullish(),
  end: instant.nullish(),
  max_runs: z.number().int().min(1).max(1_000_000).nullish(),
  grace_seconds: z.number().int().min(1).max(86400).nullish(),
};
export const scheduleSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("cron"),
    expression: z.string().min(1).max(256),
    timezone: z.string().min(1).max(100),
    ...options,
  }),
  z.object({
    kind: z.literal("every"),
    amount: z.number().int().min(1),
    unit: z.enum(["minutes", "hours", "days"]),
    anchor: instant,
    ...options,
  }),
  z.object({ kind: z.literal("at"), at: instant, ...options }),
]);
export type ScheduleSpec = z.infer<typeof scheduleSchema>;
export const deliverToSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("thread") }),
  z.object({ type: z.literal("notification") }),
  z.object({ type: z.literal("chat"), chat_id: z.string().min(1) }),
]);
export const automationFormSchema = z
  .object({
    label: z.string().trim().min(1).max(128),
    source: z.enum(["webhook", "schedule"]),
    kind: z.enum(["cron", "every", "at"]),
    expression: z.string(),
    timezone: z.string(),
    amount: z.number().int().min(1),
    unit: z.enum(["minutes", "hours", "days"]),
    anchor: z.string(),
    at: z.string(),
    start: z.string(),
    end: z.string(),
    max_runs: z.string(),
    grace_seconds: z.string(),
    delivery_type: z.enum(["assistant", "notification", "webhook", "agent"]),
    agent_id: z.string(),
    thread_policy: z.enum(["default", "home", "dedicated", "new"]),
    instruction: z.string(),
    confirmation_policy: z.enum(["changes", "destructive"]),
    deliver_type: z.enum(["thread", "chat", "notification"]),
    chat_id: z.string(),
    overlap: z.enum(["skip", "queue"]),
    webhook_url: z.string(),
    conversation_id: z.string(),
    verification_mode: z.enum(["bearer", "query", "hmac"]),
    signature_header: z.string(),
  })
  .superRefine((v, ctx) => {
    const issue = (path: keyof typeof v, message: string) =>
      ctx.addIssue({ code: "custom", path: [path], message });
    if (v.delivery_type === "assistant") {
      if (!v.agent_id) issue("agent_id", "Choose an agent");
      if (
        !v.instruction.trim() ||
        new TextEncoder().encode(v.instruction).length > 8192
      )
        issue("instruction", "Enter an instruction of up to 8 KB");
      if (v.deliver_type === "chat" && !v.chat_id)
        issue("chat_id", "Choose a chat that allows posts");
    }
    if (v.source === "schedule") {
      try {
        scheduleSchema.parse(formSchedule(v));
      } catch {
        issue(
          v.kind === "at" ? "at" : v.kind === "every" ? "anchor" : "expression",
          "Check the schedule times and limits",
        );
      }
      if (v.kind === "cron") {
        if (v.expression.trim().split(/\s+/).length !== 5)
          issue("expression", "Cron needs five fields");
        try {
          new Intl.DateTimeFormat("en", { timeZone: v.timezone });
        } catch {
          issue("timezone", "Enter an IANA timezone");
        }
      }
    }
    if (v.delivery_type === "webhook") {
      try {
        if (new URL(v.webhook_url).protocol !== "https:") throw new Error();
      } catch {
        issue("webhook_url", "Enter a public HTTPS URL");
      }
    }
    if (v.delivery_type === "agent" && !v.conversation_id)
      issue("conversation_id", "Enter the device conversation ID");
    if (
      v.source === "webhook" &&
      v.verification_mode === "hmac" &&
      !v.signature_header
    )
      issue("signature_header", "Enter a signature header");
  });
export type AutomationForm = z.infer<typeof automationFormSchema>;
export function formSchedule(v: AutomationForm): ScheduleSpec {
  const limits = {
    ...(v.start ? { start: automationInstant(v.start, v.timezone) } : {}),
    ...(v.end ? { end: automationInstant(v.end, v.timezone) } : {}),
    ...(v.max_runs ? { max_runs: Number(v.max_runs) } : {}),
    ...(v.grace_seconds ? { grace_seconds: Number(v.grace_seconds) } : {}),
  };
  switch (v.kind) {
    case "cron":
      return {
        kind: "cron",
        expression: v.expression,
        timezone: v.timezone,
        ...limits,
      };
    case "every":
      return {
        kind: "every",
        amount: v.amount,
        unit: v.unit,
        anchor: automationInstant(v.anchor, v.timezone),
        ...limits,
      };
    case "at":
      return { kind: "at", at: automationInstant(v.at, v.timezone), ...limits };
  }
}
export function automationRequest(v: AutomationForm): CreateTriggerRequest {
  return {
    label: v.label,
    source: v.source,
    ...(v.source === "schedule" ? { schedule: formSchedule(v) } : {}),
    overlap: v.overlap,
    verification:
      v.source === "schedule"
        ? { mode: "schedule" }
        : v.verification_mode === "hmac"
          ? { mode: "hmac_sha256", header_name: v.signature_header }
          : { mode: "token", location: v.verification_mode },
    delivery:
      v.delivery_type === "assistant"
        ? {
            type: "assistant",
            agent_id: v.agent_id,
            thread_policy:
              v.thread_policy === "default" ? null : v.thread_policy,
            instruction: v.instruction,
            confirmation_policy: v.confirmation_policy,
            deliver_to:
              v.deliver_type === "chat"
                ? { type: "chat", chat_id: v.chat_id }
                : { type: v.deliver_type },
          }
        : v.delivery_type === "webhook"
          ? { type: "webhook", url: v.webhook_url }
          : v.delivery_type === "agent"
            ? { type: "agent", conversation_id: v.conversation_id }
            : { type: "notification" },
  };
}
export function automationDefaults(
  timezone: string,
  row?: TriggerResponse,
): AutomationForm {
  const now = new Date(Date.now() + 3600_000).toISOString();
  const spec = row?.schedule;
  const delivery = row?.delivery;
  return {
    label: row?.label ?? "",
    source: row ? (row.source ?? "webhook") : "schedule",
    kind: spec?.kind ?? "cron",
    expression: spec?.kind === "cron" ? spec.expression : "0 8 * * 1-5",
    timezone: spec?.kind === "cron" ? spec.timezone : timezone,
    amount: spec?.kind === "every" ? spec.amount : 1,
    unit: spec?.kind === "every" ? spec.unit : "hours",
    anchor: localTime(spec?.kind === "every" ? spec.anchor : now, timezone),
    at: localTime(spec?.kind === "at" ? spec.at : now, timezone),
    start: localTime(spec?.start ?? "", timezone),
    end: localTime(spec?.end ?? "", timezone),
    max_runs: spec?.max_runs?.toString() ?? "",
    grace_seconds: spec?.grace_seconds?.toString() ?? "",
    delivery_type: delivery?.type ?? "assistant",
    agent_id: delivery?.type === "assistant" ? delivery.agent_id : "",
    thread_policy:
      delivery?.type === "assistant"
        ? (delivery.thread_policy ?? "default")
        : "default",
    instruction: delivery?.type === "assistant" ? delivery.instruction : "",
    confirmation_policy:
      delivery?.type === "assistant"
        ? (delivery.confirmation_policy ?? "changes")
        : "changes",
    deliver_type:
      delivery?.type === "assistant" ? delivery.deliver_to.type : "thread",
    chat_id:
      delivery?.type === "assistant" && delivery.deliver_to.type === "chat"
        ? delivery.deliver_to.chat_id
        : "",
    overlap: row?.overlap ?? "skip",
    webhook_url: delivery?.type === "webhook" ? delivery.url : "",
    conversation_id: delivery?.type === "agent" ? delivery.conversation_id : "",
    verification_mode:
      row?.verification.mode === "hmac_sha256"
        ? "hmac"
        : row?.verification.mode === "token"
          ? row.verification.location
          : "bearer",
    signature_header:
      row?.verification.mode === "hmac_sha256"
        ? row.verification.header_name
        : "X-Hub-Signature-256",
  };
}
export const runSchema = z.object({
  id: z.string(),
  scheduled_at: instant,
  started_at: instant.nullable(),
  finished_at: instant.nullable(),
  outcome: z.enum([
    "pending",
    "started",
    "waiting",
    "completed",
    "failed",
    "skipped",
  ]),
  missed: z
    .object({ count: z.number(), from: instant, through: instant })
    .nullish(),
  reason: z.string().nullable(),
  thread_id: z.string().nullable(),
  duration_ms: z.number().nullable(),
});
export const runsSchema = z.object({
  next_cursor: z.object({ before: instant, before_id: z.string() }).nullish(),
  runs: z.array(runSchema),
});
export type TriggerRun = z.infer<typeof runSchema>;

export const automationPreferencesSchema = z.object({
  timezone: z
    .string()
    .min(1)
    .refine((value) => {
      try {
        new Intl.DateTimeFormat("en", { timeZone: value });
        return true;
      } catch {
        return false;
      }
    }, "Enter an IANA timezone"),
  schedule_minimum_minutes: z
    .number()
    .int()
    .min(DEFAULT_SCHEDULE_MINIMUM_MINUTES)
    .max(SCHEDULE_MINIMUM_MINUTES_LIMIT),
  trigger_runs_per_hour: z
    .number()
    .int()
    .min(1)
    .max(TRIGGER_RUNS_PER_HOUR_LIMIT),
  trigger_runs_per_day: z.number().int().min(1).max(TRIGGER_RUNS_PER_DAY_LIMIT),
});
