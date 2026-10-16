import { z } from "zod";
import {
  automationCreateRequestSchema,
  automationUpdateRequestSchema,
  scheduleSchema,
  type Automation,
  type AutomationCreateRequest,
  type AutomationUpdateRequest,
  type ScheduleSpec,
} from "./automationContracts";
import { automationInstant, localTime } from "./automationTime";

const formFields = z.object({
  label: z.string(),
  source: z.enum(["webhook", "schedule"]),
  kind: z.enum(["cron", "every", "at"]),
  expression: z.string(),
  timezone: z.string(),
  amount: z.number(),
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
});
export type AutomationForm = z.infer<typeof formFields>;

/** Local calendar strings are resolved in the owner's chosen timezone. */
export function formSchedule(
  values: AutomationForm,
  row?: Automation,
): ScheduleSpec {
  const previous = row?.schedule;
  const baseline = row ? automationDefaults(values.timezone, row) : undefined;
  const instant = (
    field: "start" | "end" | "anchor" | "at",
    old?: string | null,
  ) =>
    old &&
    baseline?.[field] === values[field] &&
    baseline.timezone === values.timezone
      ? old
      : automationInstant(values[field], values.timezone);
  const options = {
    ...(values.start ? { start: instant("start", previous?.start) } : {}),
    ...(values.end ? { end: instant("end", previous?.end) } : {}),
    ...(values.max_runs ? { max_runs: Number(values.max_runs) } : {}),
    ...(values.grace_seconds
      ? { grace_seconds: Number(values.grace_seconds) }
      : {}),
  };
  const timing =
    values.kind === "cron"
      ? {
          kind: "cron" as const,
          expression: values.expression.trim(),
          timezone: values.timezone,
        }
      : values.kind === "every"
        ? {
            kind: "every" as const,
            amount: values.amount,
            unit: values.unit,
            anchor: instant(
              "anchor",
              previous?.kind === "every" ? previous.anchor : undefined,
            ),
          }
        : {
            kind: "at" as const,
            at: instant(
              "at",
              previous?.kind === "at" ? previous.at : undefined,
            ),
          };
  return scheduleSchema.parse({ ...timing, ...options });
}

function requestFields(values: AutomationForm, row?: Automation) {
  return {
    label: values.label,
    source: values.source,
    ...(values.source === "schedule"
      ? { schedule: formSchedule(values, row) }
      : {}),
    overlap: values.overlap,
    verification:
      values.source === "schedule"
        ? { mode: "schedule" as const }
        : values.verification_mode === "hmac"
          ? {
              mode: "hmac_sha256" as const,
              header_name: values.signature_header.trim(),
            }
          : { mode: "token" as const, location: values.verification_mode },
    delivery:
      values.delivery_type === "assistant"
        ? {
            type: "assistant" as const,
            agent_id: values.agent_id,
            thread_policy:
              values.thread_policy === "default" ? null : values.thread_policy,
            instruction: values.instruction,
            confirmation_policy: values.confirmation_policy,
            deliver_to:
              values.deliver_type === "chat"
                ? { type: "chat" as const, chat_id: values.chat_id }
                : { type: values.deliver_type },
          }
        : values.delivery_type === "webhook"
          ? { type: "webhook" as const, url: values.webhook_url.trim() }
          : values.delivery_type === "agent"
            ? {
                type: "agent" as const,
                conversation_id: values.conversation_id,
              }
            : { type: "notification" as const },
  };
}

export const automationFormSchema = formFields.superRefine(
  (values, context) => {
    try {
      const result = automationCreateRequestSchema.safeParse(
        requestFields(values),
      );
      if (result.success) return;
      const fields: Record<string, keyof AutomationForm> = {
        label: "label",
        agent_id: "agent_id",
        instruction: "instruction",
        url: "webhook_url",
        conversation_id: "conversation_id",
        chat_id: "chat_id",
        header_name: "signature_header",
        timezone: "timezone",
        expression: "expression",
        amount: "amount",
        anchor: "anchor",
        at: "at",
        start: "start",
        end: "end",
        max_runs: "max_runs",
        grace_seconds: "grace_seconds",
      };
      for (const issue of result.error.issues) {
        const leaf = String(issue.path.at(-1));
        context.addIssue({
          code: "custom",
          path: [fields[leaf] ?? "kind"],
          message: issue.message,
        });
      }
    } catch {
      context.addIssue({
        code: "custom",
        path: [
          values.kind === "at"
            ? "at"
            : values.kind === "every"
              ? "anchor"
              : "expression",
        ],
        message: "Check the schedule times and limits",
      });
    }
  },
);

export function automationRequest(
  values: AutomationForm,
): AutomationCreateRequest {
  return automationCreateRequestSchema.parse(
    requestFields(automationFormSchema.parse(values)),
  );
}

/** PATCH cannot change ingress verification or source. Only send edited fields,
 * keeping exact stored timestamps (including seconds and DST folds) intact. */
export function automationUpdateRequest(
  values: AutomationForm,
  row: Automation,
): AutomationUpdateRequest {
  if (values.source !== (row.source ?? "webhook"))
    throw new Error("An existing automation's source cannot be changed");
  const parsed = automationFormSchema.parse(values);
  const baseline = automationDefaults(values.timezone, row);
  const request = requestFields(parsed, row);
  const changed = (fields: (keyof AutomationForm)[]) =>
    fields.some((field) => parsed[field] !== baseline[field]);
  const scheduleFields: (keyof AutomationForm)[] = [
    "kind",
    "start",
    "end",
    "max_runs",
    "grace_seconds",
    "timezone",
  ];
  if (parsed.kind === "cron") scheduleFields.push("expression");
  else if (parsed.kind === "every")
    scheduleFields.push("amount", "unit", "anchor");
  else scheduleFields.push("at");
  const deliveryFields: (keyof AutomationForm)[] = ["delivery_type"];
  if (parsed.delivery_type === "assistant")
    deliveryFields.push(
      "agent_id",
      "thread_policy",
      "instruction",
      "confirmation_policy",
      "deliver_type",
      "chat_id",
    );
  else if (parsed.delivery_type === "webhook")
    deliveryFields.push("webhook_url");
  else if (parsed.delivery_type === "agent")
    deliveryFields.push("conversation_id");
  // Schedule options are a replacement object: explicit null clears a limit.
  const schedule = request.schedule && {
    ...request.schedule,
    start: request.schedule.start ?? null,
    end: request.schedule.end ?? null,
    max_runs: request.schedule.max_runs ?? null,
    grace_seconds: request.schedule.grace_seconds ?? null,
  };
  return automationUpdateRequestSchema.parse({
    ...(request.label.trim() !== row.label ? { label: request.label } : {}),
    ...(changed(deliveryFields) ? { delivery: request.delivery } : {}),
    ...(parsed.source === "schedule" && changed(scheduleFields)
      ? { schedule }
      : {}),
    ...(parsed.overlap !== (row.overlap ?? "skip")
      ? { overlap: parsed.overlap }
      : {}),
  });
}

export function automationDefaults(
  timezone: string,
  row?: Automation,
): AutomationForm {
  const now = new Date(Date.now() + 3_600_000).toISOString();
  const spec = row?.schedule;
  const delivery = row?.delivery;
  const zone = spec?.kind === "cron" ? spec.timezone : timezone;
  return {
    label: row?.label ?? "",
    source: row ? (row.source ?? "webhook") : "schedule",
    kind: spec?.kind ?? "cron",
    expression: spec?.kind === "cron" ? spec.expression : "0 8 * * 1-5",
    timezone: zone,
    amount: spec?.kind === "every" ? spec.amount : 1,
    unit: spec?.kind === "every" ? spec.unit : "hours",
    anchor: localTime(spec?.kind === "every" ? spec.anchor : now, zone),
    at: localTime(spec?.kind === "at" ? spec.at : now, zone),
    start: localTime(spec?.start ?? "", zone),
    end: localTime(spec?.end ?? "", zone),
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

export function automationScheduleSummary(row: Automation): string {
  const schedule = row.schedule;
  if (!schedule || (row.source ?? "webhook") === "webhook")
    return "Webhook event";
  if (schedule.kind === "cron")
    return `${schedule.expression} · ${schedule.timezone}`;
  if (schedule.kind === "every")
    return `Every ${schedule.amount} ${schedule.unit}`;
  return "One time";
}
