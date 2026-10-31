import { z } from "zod";

export const collaborationGroupSchema = z.object({
  version: z.literal(1),
  creator_user_id: z.string(),
  membership_revision: z.number().int().positive(),
  history_disclosure: z.string(),
  can_manage: z.boolean(),
  your_request_ids: z.array(z.string()),
  transfer_to_user_id: z.string().nullable(),
  guest_access: z.array(
    z.object({
      agent_id: z.string(),
      enabled: z.boolean(),
      revision: z.number().int(),
      can_enable: z.boolean(),
      can_disable: z.boolean(),
      lane: z.enum(["owner", "org_member", "shared", "guest"]).nullable(),
    }),
  ),
  proposals: z.array(
    z.object({ id: z.string(), agent_id: z.string(), proposed_by: z.string() }),
  ),
  invites: z.array(z.object({ id: z.string(), expires_at: z.string() })),
});
export const collaborationDecisionSchema = z.object({
  id: z.string().optional(),
  kind: z.enum(["action", "consent", "approval"]).optional(),
  requester_id: z.string(),
  agent_id: z.string(),
  can_decide: z.boolean(),
  summary: z.string(),
  connection_id: z.string().optional(),
  connection_name: z.string().nullable().optional(),
  audience: z.string().optional(),
});
export type CollaborationDecision = z.infer<typeof collaborationDecisionSchema>;
export const groupInviteSchema = z.object({
  id: z.string(),
  token: z.string(),
  expires_at: z.string(),
  history_disclosure: z.string(),
});
export const groupInvitePreviewSchema = z.object({
  group_id: z.string(),
  group_name: z.string(),
  history_disclosure: z.string(),
  expires_at: z.string(),
  creator_name: z.string().optional(),
  participants: z
    .array(z.object({ id: z.string(), display_name: z.string() }))
    .optional(),
});
export type CollaborationChange =
  | { action: "invite" | "leave" | "delete" | "accept_transfer" }
  | { action: "cancel_invite"; invite_id: string }
  | { action: "quick_add"; person_id: string; history_disclosed: boolean }
  | { action: "remove" | "propose_transfer"; person_id: string }
  | {
      action: "add_agent" | "propose_agent" | "remove_agent" | "lead";
      agent_id: string;
    }
  | { action: "accept_agent"; proposal_id: string }
  | {
      action: "guest_access";
      agent_id: string;
      enabled: boolean;
      guest_revision: number;
    }
  | { action: "rename"; name: string };
