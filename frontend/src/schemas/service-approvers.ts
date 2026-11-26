import { z } from "zod";

export const MAX_SERVICE_APPROVERS = 128;

export const serviceApproversDraft = z.object({
  approver_user_ids: z.array(z.uuid()).min(1).max(MAX_SERVICE_APPROVERS).nullable(),
});
export type ServiceApproversDraft = z.infer<typeof serviceApproversDraft>;

export type ApproverIneligibility = "not_admin" | "not_member" | "out_of_scope" | "inactive" | "unknown";

export interface ServiceApprovers extends ServiceApproversDraft {
  readonly config_id: string;
  readonly revision: number;
  readonly authoring_available: boolean;
  readonly authoring_unavailable_reason?: "flag_off" | "readiness_missing" | null;
  readonly unavailable?: readonly {
    user_id: string;
    display_name: string | null;
    reason: ApproverIneligibility;
  }[];
  readonly eligible_candidates: readonly { user_id: string; display_name: string | null }[];
}
