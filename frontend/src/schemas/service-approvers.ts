import { z } from "zod";

export const MAX_SERVICE_APPROVERS = 128;

export const serviceApproversDraft = z.object({
  approver_user_ids: z.array(z.uuid()).min(1).max(MAX_SERVICE_APPROVERS).nullable(),
});
export type ServiceApproversDraft = z.infer<typeof serviceApproversDraft>;

export interface ServiceApprovers extends ServiceApproversDraft {
  readonly config_id: string;
  readonly revision: number;
  readonly authoring_available: boolean;
  readonly eligible_candidates: readonly { user_id: string; display_name: string | null }[];
}
