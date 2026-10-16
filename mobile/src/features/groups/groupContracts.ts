import { z } from "zod";
import {
  attachmentSchema,
  conversationIdSchema,
} from "../assistant/assistantTypes";
import { MAX_ASSISTANT_ATTACHMENTS } from "../assistant/assistantUploads";

export const MAX_GROUP_MEMBERS = 8;
export const MAX_GROUP_PARTICIPANTS = 16;
export const MAX_GROUP_NAME = 60;
export const MAX_GROUP_MESSAGE = 32_768;
export const GROUP_FEEDBACK_COMMENT_MAX = 1000;
export const GROUP_MAX_MEMBERS = MAX_GROUP_MEMBERS;
export const GROUP_MAX_PARTICIPANTS = MAX_GROUP_PARTICIPANTS;
export const GROUP_NAME_MAX = MAX_GROUP_NAME;

export const groupIdSchema = z.string().regex(/^nyxg-[a-fA-F0-9]{32}$/);
export const groupReferenceSchema = z
  .string()
  .min(1)
  .max(256)
  .refine(
    (value) => !/[\s\u0000-\u001f\u007f]/.test(value),
    "Enter a valid reference",
  );
const instant = z.string().datetime({ offset: true });
const person = z.object({ id: z.string().uuid(), display_name: z.string() });
const agent = z.object({
  id: groupReferenceSchema,
  name: z.string(),
  display_name: z.string().nullish(),
  kind: z.enum(["nyxbot", "specialist"]),
});
export const groupMemberSchema = agent.extend({
  destroyed: z.boolean().default(false),
  working: z.boolean().default(false),
});
export const groupSchema = z.object({
  owner: z
    .object({
      type: z.enum(["personal", "org"]),
      id: z.string().uuid(),
      name: z.string(),
    })
    .optional(),
  participants: z.array(person).optional(),
  your_role: z.enum(["creator", "admin", "participant"]).optional(),
  id: groupIdSchema,
  name: z.string(),
  members: z.array(groupMemberSchema),
  lead_agent_id: groupReferenceSchema,
  default_recipient_ids: z.array(groupReferenceSchema).optional(),
  working_agent_ids: z.array(groupReferenceSchema).default([]),
  message_count: z.number().int().nonnegative().default(0),
  last_message_at: instant.nullable().default(null),
  created_at: instant,
});
export const groupListSchema = z.object({ groups: z.array(groupSchema) });
const activity = z.object({
  id: z.string(),
  label: z.string(),
  status: z.enum(["running", "completed", "error"]),
  started_at: instant,
  ended_at: instant.nullish(),
  machine: z.record(z.string(), z.unknown()).nullish(),
});
export const groupFeedbackSchema = z.object({
  vote: z.enum(["up", "down"]),
  comment: z.string().nullable(),
  updated_at: instant,
});
export const groupFeedbackInputSchema = z
  .object({
    vote: z.enum(["up", "down"]),
    // The server trims and removes controls/bidi characters before bounding
    // the comment. Preserve omission: it retains an existing comment, while
    // an explicit empty string clears it.
    comment: z
      .string()
      .refine((value) => {
        const sanitized = value
          .trim()
          .replace(
            /[\p{Cc}\u200b\u200c\u200d\u200e\u200f\u202a\u202b\u202c\u202d\u202e\u2060\u2066\u2067\u2069]/gu,
            "",
          );
        return [...sanitized].length <= GROUP_FEEDBACK_COMMENT_MAX;
      }, "Keep feedback under 1,000 characters.")
      .optional(),
  })
  .strict();
export const groupMessageSchema = z.object({
  feedback: groupFeedbackSchema
    .partial({ comment: true, updated_at: true })
    .nullish(),
  activities: z.array(activity).default([]),
  author: person.optional(),
  attachments: z.array(attachmentSchema).default([]),
  id: z.string().uuid(),
  seq: z.number().int().positive(),
  role: z.enum(["user", "agent", "notice"]),
  agent: agent.nullable().default(null),
  text: z.string(),
  recipient_agent_ids: z.array(groupReferenceSchema).optional(),
  created_at: instant,
});
export const groupPendingActionSchema = z.object({
  conversation_id: conversationIdSchema,
  acknowledgement_id: z.string().uuid().optional(),
  approval_request_id: z.string().uuid().optional(),
  agent_id: groupReferenceSchema.nullish(),
  summary: z.string(),
  confirm_phrase: z
    .string()
    .regex(/^yes \d{4}$/)
    .optional(),
  triggering_person: person.optional(),
  can_decide: z.boolean().optional(),
  authored_skill: z.boolean().optional(),
  review_url: z.string().optional(),
  expires_at: instant,
});
export const groupMessagesPageSchema = z.object({
  group: groupSchema,
  pending_actions: z.array(groupPendingActionSchema).default([]),
  messages: z.array(groupMessageSchema),
  before_seq: z.number().int().positive().nullable().default(null),
});
export const groupPostedSchema = z.object({
  message: groupMessageSchema,
  addressed_agent_ids: z.array(groupReferenceSchema).default([]),
});

const groupName = z
  .string()
  .trim()
  .refine(
    (value) => [...value].length >= 1 && [...value].length <= MAX_GROUP_NAME,
    "Use a group name between 1 and 60 characters.",
  );
const distinct = (values: readonly string[]) =>
  new Set(values).size === values.length;
const members = z
  .array(groupReferenceSchema)
  .min(1, "Pick at least one agent.")
  .max(MAX_GROUP_MEMBERS, "A group has at most 8 agents.")
  .refine(distinct, "Choose each agent only once.");
const participants = z
  .array(z.string().uuid())
  .max(MAX_GROUP_PARTICIPANTS)
  .refine(distinct, "Choose each person only once.");

/** Create intentionally has no lead_agent_id: the backend rejects it. */
export const groupCreateSchema = z
  .object({
    org: groupReferenceSchema.optional(),
    participant_user_ids: participants.optional(),
    name: groupName,
    member_agent_ids: members,
  })
  .strict()
  .superRefine((value, context) => {
    if (!value.org && value.participant_user_ids?.length)
      context.addIssue({
        code: "custom",
        path: ["participant_user_ids"],
        message: "Participants require an organization group.",
      });
  });
export const groupUpdateSchema = z
  .object({
    participant_user_ids: participants
      .min(1, "Keep at least one participant.")
      .optional(),
    lead_agent_id: groupReferenceSchema.optional(),
    leave: z.boolean().optional(),
    name: groupName.optional(),
    member_agent_ids: members.optional(),
  })
  .strict()
  .superRefine((value, context) => {
    if (
      value.leave &&
      Object.entries(value).some(
        ([key, entry]) => key !== "leave" && entry !== undefined,
      )
    ) {
      context.addIssue({
        code: "custom",
        message: "Leave the group separately from editing its settings.",
      });
    }
    if (
      value.lead_agent_id &&
      value.member_agent_ids &&
      !value.member_agent_ids.includes(value.lead_agent_id)
    ) {
      context.addIssue({
        code: "custom",
        path: ["lead_agent_id"],
        message: "The lead must be a selected agent.",
      });
    }
  });
export const groupPostRequestSchema = z
  .object({
    text: z
      .string()
      .trim()
      .refine(
        (value) => [...value].length <= MAX_GROUP_MESSAGE,
        "Keep the message under 32,768 characters.",
      ),
    attachment_ids: z
      .array(z.string().uuid())
      .max(MAX_ASSISTANT_ATTACHMENTS)
      .refine(distinct)
      .optional(),
    recipient_agent_ids: z
      .array(groupReferenceSchema)
      .min(1)
      .max(MAX_GROUP_MEMBERS)
      .refine(distinct)
      .optional(),
  })
  .strict()
  .superRefine((value, context) => {
    if (!value.text && !value.attachment_ids?.length)
      context.addIssue({
        code: "custom",
        path: ["text"],
        message: "Enter a message or attach a file.",
      });
  });

const orgRole = z.enum(["owner", "admin", "member", "viewer"]);
export const groupOrganizationSchema = z.object({
  id: z.string().uuid(),
  slug: groupReferenceSchema,
  display_name: z.string().nullish(),
  your_role: orgRole,
});
export const groupOrganizationMemberSchema = z.object({
  membership_id: z.string().uuid(),
  user_id: z.string().uuid(),
  display_name: z.string().nullish(),
  email: z.string().nullish(),
  role: orgRole,
  revoked_at: instant.nullable(),
});
export const groupIdentitySchema = z
  .object({
    id: z.string().uuid(),
    capabilities: z
      .object({ enabled_features: z.array(z.string()).default([]) })
      .optional(),
  })
  .transform((value) => ({
    id: value.id,
    enabled_features: value.capabilities?.enabled_features ?? [],
  }));

export type Group = z.infer<typeof groupSchema>;
export type GroupMember = z.infer<typeof groupMemberSchema>;
export type GroupMessage = z.infer<typeof groupMessageSchema>;
export type GroupPendingAction = z.infer<typeof groupPendingActionSchema>;
export type GroupMessagesPage = z.infer<typeof groupMessagesPageSchema>;
export type GroupPosted = z.infer<typeof groupPostedSchema>;
export type GroupCreate = z.infer<typeof groupCreateSchema>;
export type GroupUpdate = z.infer<typeof groupUpdateSchema>;
export type GroupPostRequest = z.infer<typeof groupPostRequestSchema>;
export type GroupOrganization = z.infer<typeof groupOrganizationSchema>;
export type GroupOrganizationMember = z.infer<
  typeof groupOrganizationMemberSchema
>;
export type GroupIdentity = z.infer<typeof groupIdentitySchema>;
export type GroupFeedback = z.infer<typeof groupFeedbackSchema>;
export type GroupFeedbackInput = z.infer<typeof groupFeedbackInputSchema>;
export type GroupOrg = GroupOrganization;
export type GroupOrgMember = GroupOrganizationMember;
