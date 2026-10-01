import { z } from "zod";
import {
  scheduleSchema,
  deliverToSchema,
  runSchema,
  type ScheduleSpec,
} from "./automations";

export const triggerStatusSchema = z.enum(["active", "disabled"]);

export const triggerVerificationSchema = z.discriminatedUnion("mode", [
  z.object({ mode: z.literal("schedule") }),
  z.object({
    mode: z.literal("token"),
    location: z.enum(["bearer", "query"]),
  }),
  z.object({
    mode: z.literal("hmac_sha256"),
    header_name: z.string().min(1).max(128),
  }),
]);

export const triggerDeliverySchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("assistant"),
    confirmation_policy: z.enum(["changes", "destructive"]).optional(),
    agent_id: z.string(),
    thread_policy: z.enum(["home", "dedicated", "new"]).nullable().optional(),
    instruction: z.string(),
    deliver_to: deliverToSchema,
  }),
  z.object({ type: z.literal("webhook"), url: z.string().url() }),
  z.object({ type: z.literal("agent"), conversation_id: z.string().min(1) }),
  z.object({ type: z.literal("notification") }),
]);

export const triggerResponseSchema = z.object({
  source: z.enum(["webhook", "schedule"]).optional(),
  last_run: runSchema.nullable().optional(),
  schedule: scheduleSchema.nullable().optional(),
  overlap: z.enum(["skip", "queue"]).optional(),
  next_runs: z.array(z.string()).optional(),
  next_run_at: z.string().nullable().optional(),
  pause_reason: z.string().nullable().optional(),
  id: z.string().uuid(),
  user_id: z.string().min(1),
  label: z.string().min(1),
  user_service_id: z.string().nullable(),
  status: triggerStatusSchema,
  verification: triggerVerificationSchema,
  delivery: triggerDeliverySchema,
  inbound_url: z.string().url().nullable(),
  created_at: z.string().datetime({ offset: true }),
  updated_at: z.string().datetime({ offset: true }),
});

export const createTriggerResponseSchema = z.object({
  trigger: triggerResponseSchema,
  secret: z.string().min(1).nullable(),
  delivery_signing_secret: z.string().nullable(),
});

export const updateTriggerResponseSchema = z.object({
  trigger: triggerResponseSchema,
  delivery_signing_secret: z.string().nullable(),
});

export const listTriggersResponseSchema = z.object({
  triggers: z.array(triggerResponseSchema),
});

export const rotateTriggerSecretResponseSchema = z.object({
  trigger: triggerResponseSchema,
  secret: z.string().min(1),
});

export const deleteTriggerResponseSchema = z.object({
  message: z.string(),
});

export const triggerFormSchema = z
  .object({
    label: z.string().trim().min(1, "Label is required").max(128),
    verification_mode: z.enum(["bearer", "query", "hmac"]),
    signature_header: z.string().trim().max(128),
    delivery_type: z.enum(["webhook", "agent", "notification"]),
    webhook_url: z.string().trim(),
    conversation_id: z.string().trim(),
  })
  .superRefine((values, context) => {
    if (values.verification_mode === "hmac") {
      if (!values.signature_header) {
        context.addIssue({
          code: "custom",
          path: ["signature_header"],
          message: "Signature header is required",
        });
      } else {
        try {
          new Headers({ [values.signature_header]: "value" });
        } catch {
          context.addIssue({
            code: "custom",
            path: ["signature_header"],
            message: "Enter a valid HTTP header name",
          });
        }
      }
    }
    if (values.delivery_type === "webhook") {
      let validHttpsUrl = false;
      try {
        validHttpsUrl = new URL(values.webhook_url).protocol === "https:";
      } catch {
        validHttpsUrl = false;
      }
      if (!validHttpsUrl) {
        context.addIssue({
          code: "custom",
          path: ["webhook_url"],
          message: "Webhook URL must be a valid HTTPS URL",
        });
      }
    }
    if (values.delivery_type === "agent" && !values.conversation_id) {
      context.addIssue({
        code: "custom",
        path: ["conversation_id"],
        message: "Conversation ID is required",
      });
    }
  });

export type TriggerStatus = z.infer<typeof triggerStatusSchema>;
export type TriggerVerification = z.infer<typeof triggerVerificationSchema>;
export type TriggerDelivery = z.infer<typeof triggerDeliverySchema>;
export type TriggerResponse = z.infer<typeof triggerResponseSchema>;
export type CreateTriggerResponse = z.infer<typeof createTriggerResponseSchema>;
export type UpdateTriggerResponse = z.infer<typeof updateTriggerResponseSchema>;
export type ListTriggersResponse = z.infer<typeof listTriggersResponseSchema>;
export type RotateTriggerSecretResponse = z.infer<
  typeof rotateTriggerSecretResponseSchema
>;
export type DeleteTriggerResponse = z.infer<typeof deleteTriggerResponseSchema>;
export type TriggerForm = z.infer<typeof triggerFormSchema>;

export interface CreateTriggerRequest {
  readonly source?: "webhook" | "schedule";
  readonly schedule?: ScheduleSpec;
  readonly overlap?: "skip" | "queue";
  readonly watch_id?: string;
  readonly label: string;
  readonly user_service_id?: string;
  readonly verification: TriggerVerification;
  readonly delivery: TriggerDelivery;
  readonly target_org_id?: string;
}

export interface UpdateTriggerRequest {
  readonly schedule?: ScheduleSpec;
  readonly overlap?: "skip" | "queue";
  readonly label?: string;
  readonly status?: TriggerStatus;
  readonly delivery?: TriggerDelivery;
}

export function buildCreateTriggerRequest(
  values: TriggerForm,
): CreateTriggerRequest {
  const verification: TriggerVerification =
    values.verification_mode === "hmac"
      ? {
          mode: "hmac_sha256",
          header_name: values.signature_header,
        }
      : {
          mode: "token",
          location: values.verification_mode,
        };
  const delivery: TriggerDelivery =
    values.delivery_type === "webhook"
      ? { type: "webhook", url: values.webhook_url }
      : values.delivery_type === "agent"
        ? { type: "agent", conversation_id: values.conversation_id }
        : { type: "notification" };
  return { label: values.label, verification, delivery };
}
