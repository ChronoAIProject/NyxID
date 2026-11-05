import { z } from "zod";

export const conversationIdSchema = z.string().regex(/^nyxa-[a-f0-9]{32}$/);
export const activitySchema = z.object({
  id: z.string(),
  label: z.string(),
  status: z.enum(["running", "completed", "error"]),
  started_at: z.string(),
  ended_at: z.string().nullish(),
  failure: z
    .object({
      kind: z.enum(["service_status", "tool_error", "refused", "internal"]),
      status: z.number().int().nullish(),
    })
    .nullish(),
  machine: z
    .object({
      node_id: z.string(),
      machine_name: z.string().nullish(),
      context_mode: z.enum(["shared_legacy", "separated"]).nullish(),
      action: z.string(),
      status: z.string(),
      error_code: z.number().nullish(),
      operation_id: z.string().optional(),
      agent_id: z.string().optional(),
      job_id: z.string().nullish(),
      exit_code: z.number().nullish(),
      bytes: z.number().nonnegative().nullish(),
      duration_ms: z.number().nonnegative().nullish(),
      screenshot_id: z.string().nullish(),
      preview_id: z.string().nullish(),
      preview_enabled: z.boolean().optional(),
    })
    .nullish(),
});
export const attachmentSchema = z.object({
  id: z.string(),
  content_type: z.string(),
  size: z.number(),
  label: z.string(),
  origin: z.string().optional(),
  pages: z.number().int().nonnegative().nullish(),
  expired: z.boolean().optional(),
  image_input: z.string().nullish(),
});
export type AssistantAttachment = z.infer<typeof attachmentSchema>;
export const conversationSchema = z.object({
  id: conversationIdSchema,
  title: z.string(),
  model: z.string(),
  created_at: z.string(),
  last_message_at: z.string(),
  message_count: z.number(),
  pending_acknowledgements: z.number().default(0),
  pending_events: z.number().int().nonnegative().optional(),
  active_turn: z
    .object({
      turn_id: z.string(),
      started_at: z.string(),
      stop_requested: z.boolean().optional(),
      steering_allowed: z.boolean().optional(),
      continuations: z.number().optional(),
      reply: z
        .object({
          message_id: z.string(),
          block_id: z.string(),
          cursor: z.number().int().nonnegative(),
          text: z.string(),
        })
        .nullish(),
      queued: z.boolean().optional(),
      waiting_for_capacity: z.boolean().optional(),
      capacity_wait_note_origin: z.string().nullish(),
      engine_request_started_at: z.string().nullish(),
      capacity_wait_started_at: z.string().nullish(),
      capacity_wait_deadline: z.string().nullish(),
      activity_total: z.number().int().nonnegative().nullish(),
      activities: z.array(activitySchema).default([]),
      attachments: z.array(attachmentSchema).default([]),
    })
    .nullable(),
  context_reset_reason: z.string().nullish(),
  context_reset_at: z.string().nullish(),
  role: z.enum(["orchestrator", "subagent"]).default("orchestrator"),
  agent: z
    .object({
      id: z.string(),
      name: z.string(),
      display_name: z.string().nullish(),
      kind: z.enum(["nyxbot", "specialist"]),
      destroyed: z.boolean().default(false),
    })
    .nullish(),
  channel: z
    .object({
      platform: z.string(),
      channel_agent_id: z.string().nullish(),
      bot_label: z.string().nullish(),
      chat_id: z.string().nullish(),
      chat_kind: z.string().nullish(),
      chat_title: z.string().nullish(),
      parent_chat_id: z.string().nullish(),
      thread_id: z.string().nullish(),
      parent_title: z.string().nullish(),
    })
    .nullish(),
});
export const recoverySchema = z.object({
  status: z.enum(["available", "review"]),
  reason: z.string(),
  text: z.string().nullable(),
  attachments: z.array(z.string()),
});
const failureReasonSchema = z.object({
  source: z.string(),
  stage: z.string(),
  code: z.string(),
  http_status: z.number().int().nullish(),
  nyxid_error_code: z.number().int().nullish(),
});
const turnMetadataSchema = z.object({
  attempts: z.number().int().nonnegative().nullish(),
  capacity_wait_ms: z.number().nonnegative().nullish(),
  failure_reason: failureReasonSchema.nullish(),
  channel_delivery: z
    .object({
      status: z.string(),
      heard_prefix_chars: z.number().int().nonnegative().nullish(),
      heard_prefix_matches_sent: z.boolean(),
    })
    .nullish(),
});
export const feedbackSchema = z.object({
  vote: z.enum(["up", "down"]),
  comment: z.string().nullish(),
  updated_at: z.string().optional(),
});
export const messageSchema = z.object({
  feedback: feedbackSchema.nullish(),
  id: z.string(),
  seq: z.number().int().positive(),
  turn_id: z.string(),
  role: z.enum(["user", "assistant", "orchestrator", "event"]),
  origin: z.string().nullish(),
  automation: z
    .object({
      trigger_id: z.string(),
      run_id: z.string(),
      label: z.string().nullish(),
    })
    .nullish(),
  text: z.string(),
  status: z.enum(["completed", "failed"]),
  error_code: z.string().nullable(),
  created_at: z.string(),
  activities: z.array(activitySchema).default([]),
  attachments: z.array(attachmentSchema).default([]),
  via: z.string().nullish(),
  steering: z
    .object({
      outcome: z.string(),
      code: z.string().nullish(),
      client_request_id: z.string().optional(),
    })
    .nullish(),
  continuation_of: z.string().nullish(),
  recovery: recoverySchema.nullish(),
  upstream_error_code: z.string().nullish(),
  activity_total: z.number().int().nonnegative().nullish(),
  turn_metadata: turnMetadataSchema.nullish(),
  error_detail: z
    .object({
      attempts: z.number().int().nonnegative().nullish(),
      capacity_wait_ms: z.number().nonnegative().nullish(),
      stage: z.string(),
      http_status: z.number().int().nullable(),
      upstream_request_id: z.string().nullable(),
      response_id: z.string().nullable(),
      continuations: z.number().int().nonnegative(),
      turn_id: z.string(),
      occurred_at: z.string(),
    })
    .nullish(),
});
export const acknowledgementSchema = z.object({
  creation_turn_id: z.string().nullish(),
  activity_id: z.string().nullish(),
  link: z.string().nullish(),
  decided_at: z.string().nullish(),
  continuation_receipt_id: z.string().uuid().nullish(),
  id: z.string().uuid(),
  kind: z.enum([
    "service",
    "account",
    "action",
    "operations",
    "skills",
    "machine",
    "saved_login",
  ]),
  status: z.enum(["pending", "allowed", "denied", "expired", "used"]),
  summary: z.string(),
  decider: z.enum(["user", "orchestrator"]).default("user"),
  decided_by: z.string().nullish(),
  reason: z.string().nullish(),
  service_slug: z.string().nullable(),
  service_name: z.string().nullable(),
  tool_name: z.string().nullable(),
  created_at: z.string(),
  expires_at: z.string(),
  confirmation: z
    .object({
      risk: z.enum(["read", "change", "sensitive", "destructive"]),
      reason: z.string(),
      access_mode: z.enum(["autopilot", "nyxmode", "ask_every_time"]).nullish(),
    })
    .nullish(),
  continuation_owner: z.literal("server").nullish(),
  trigger_run_id: z.string().nullish(),
  authored_skill: z
    .object({
      agent_id: z.string(),
      proposal_id: z.string(),
      revision: z.number().int().nonnegative().optional(),
      skills_revision: z.number().int().nonnegative().optional(),
    })
    .nullish(),
});
export const cmaOperationSchema = z.object({
  id: z.string(),
  turn_id: z.string(),
  kind: z.enum(["cma_provisioning", "cma_response", "cma_agent"]),
  operation_id: z.string().nullable(),
  agent_id: z.string().nullable(),
  idempotency_key: z.string().nullable(),
  status: z.string().nullable(),
  phase: z.string().nullable(),
  state: z.string(),
  reason: z.string().nullable(),
  monitor_name: z.string(),
  created_at: z.string(),
  checked_at: z.string().nullable(),
});
export type CmaOperation = z.infer<typeof cmaOperationSchema>;
export const historySchema = z.object({
  cma_operations: z.array(cmaOperationSchema).optional(),
  conversation: conversationSchema,
  messages: z.array(messageSchema),
  acknowledgements: z.array(acknowledgementSchema).default([]),
  approvals: z
    .array(
      z.object({
        id: z.string(),
        service_name: z.string(),
        summary: z.string(),
        expires_at: z.string(),
        service_slug: z.string().optional(),
        approval_mode: z.enum(["per_request", "grant"]).optional(),
        agent_key_prefix: z.string().optional(),
        created_at: z.string().optional(),
      }),
    )
    .default([]),
  waiting: z
    .array(
      z.object({
        title: z.string(),
        detail: z.string().nullish(),
        kind: z.string().optional(),
        since: z.string().optional(),
        expires_at: z.string().nullish(),
      }),
    )
    .default([]),
  before_seq: z.number().int().positive().nullable(),
});
export const indexSchema = z.object({
  conversations: z.array(conversationSchema),
  next_cursor: z.string().nullable(),
});
export const assistantCapabilitiesSchema = z.object({
  steer: z
    .object({
      max_chars: z.number().int().positive(),
      max_per_turn: z.number().int().positive(),
    })
    .nullable(),
});
export const assistantSteeringSchema = z.object({
  client_request_id: z.string(),
  outcome: z.enum(["applied", "may_not_have_applied", "refused"]),
  code: z.string().nullable(),
});
const block = z.object({
  type: z.literal("text"),
  block_id: z.string(),
  text: z.string(),
});
const cursor = { cursor: z.number().int().positive() };
export const turnEventSchema = z.discriminatedUnion("event", [
  z.object({
    ...cursor,
    event: z.literal("turn.status"),
    conversation_id: conversationIdSchema,
    turn_id: z.string(),
    status: z.enum(["running", "waiting"]),
    started_at: z.string().optional(),
    queued: z.boolean().optional(),
    waiting_for_capacity: z.boolean().optional(),
    engine_accepted: z.boolean().optional(),
    capacity_wait_note_origin: z.string().nullish(),
    engine_request_started_at: z.string().nullish(),
    capacity_wait_started_at: z.string().nullish(),
    capacity_wait_deadline: z.string().nullish(),
  }),
  z.object({
    ...cursor,
    event: z.literal("turn.continuing"),
    turn_id: z.string(),
    continuation: z.number().int().positive(),
  }),
  z.object({
    ...cursor,
    event: z.literal("turn.notice"),
    code: z.string(),
    message: z.string(),
  }),
  z.object({
    ...cursor,
    event: z.literal("message.started"),
    message_id: z.string(),
    role: z.literal("assistant"),
  }),
  z.object({
    ...cursor,
    event: z.literal("block.started"),
    message_id: z.string(),
    block_id: z.string(),
    index: z.number().int().nonnegative(),
    block,
  }),
  z.object({
    ...cursor,
    event: z.literal("block.delta"),
    block_id: z.string(),
    text: z.string(),
  }),
  z.object({
    ...cursor,
    event: z.literal("block.completed"),
    block_id: z.string(),
    block,
  }),
  z.object({
    ...cursor,
    event: z.literal("message.completed"),
    message_id: z.string(),
  }),
  z.object({
    ...cursor,
    event: z.literal("turn.completed"),
    turn_id: z.string(),
    status: z.enum(["completed", "failed", "cancelled"]),
    turn_metadata: turnMetadataSchema.nullish(),
    error: z
      .object({
        code: z.string(),
        message: z.string(),
        attempts: z.number().int().nonnegative().nullish(),
        capacity_wait_ms: z.number().nonnegative().nullish(),
        upstream_error_code: z.string().nullish(),
        stage: z.string().optional(),
        http_status: z.number().nullish(),
        upstream_request_id: z.string().nullish(),
        response_id: z.string().nullish(),
      })
      .nullable(),
  }),
]);
export type AssistantConversation = z.infer<typeof conversationSchema>;
export type AssistantHistory = z.infer<typeof historySchema>;
export type AssistantMessage = z.infer<typeof messageSchema>;
export type AssistantAcknowledgement = z.infer<typeof acknowledgementSchema>;
export type AssistantTurnEvent = z.infer<typeof turnEventSchema>;

export const assistantContextSchema = z.object({
  conversation_id: conversationIdSchema,
  agent_id: z.string().nullish(),
  binding_state: z.string(),
  reset_reason: z.string().nullish(),
  reset_at: z.string().nullish(),
  active_turn_id: z.string().nullish(),
  pending_actions: z.array(z.string()).default([]),
  checkpoint: z
    .object({
      turn_id: z.string(),
      status: z.string(),
      error_code: z.string().nullish(),
      goal: z.string().nullish(),
      completed_actions: z
        .array(z.object({ label: z.string(), turn_id: z.string() }))
        .default([]),
      failed_actions: z
        .array(z.object({ label: z.string(), turn_id: z.string() }))
        .default([]),
      output: z.string().nullish(),
      last_successful_turn_id: z.string().nullish(),
      last_successful_at: z.string().nullish(),
      updated_at: z.string(),
    })
    .nullish(),
});
