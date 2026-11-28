import { z } from "zod";

const proposal = z.object({
  version: z.number().int().min(1).max(5),
  state: z.enum(["pending", "queued", "preparing", "dispatched", "completed", "denied", "expired", "not_sent", "unknown", "failed", "unsupported"]),
  supported: z.boolean(),
  can_decide: z.boolean(),
  review_path: z.string(),
});

/** Preserve unknown proposals as non-decidable metadata, never ordinary approvals. */
export const assistantExecutionProposalSchema = z.unknown().transform((value) => {
  if (value === undefined || value === null) return value;
  const result = proposal.safeParse(value);
  return result.success ? result.data : {
    version: 0, state: "unsupported", supported: false, can_decide: false, review_path: "",
  };
});

type ReviewApproval = {
  id: string;
  execution_proposal?: z.infer<typeof assistantExecutionProposalSchema>;
  review_path?: string | null;
};

/** Only the server's exact first-party review destination is usable. */
export function assistantApprovalReviewPath(approval: ReviewApproval): string | undefined {
  const proposal = approval.execution_proposal;
  if (!proposal?.supported || proposal.state === "unsupported") return;
  const path = proposal.review_path;
  if (approval.review_path != null && approval.review_path !== path) return;
  // v2 sources intentionally review through the approvals inbox.
  if (proposal.version === 2 && path === "/assistant/approvals") return path;
  if (!path.startsWith("/assistant?")) return;
  if (/[\\#\u0000-\u0020]/.test(path)) return;
  const url = new URL(path, "https://nyxid.invalid");
  const params = url.searchParams;
  // An exact private-thread link must retain this request's card identity.
  if (proposal.version !== 2 && url.pathname === "/assistant" && /^nyxa-[a-f0-9]{32}$/.test(params.get("c") ?? "") &&
    params.get("card") === approval.id && [...params.keys()].every((key) => ["c", "card"].includes(key)) &&
    params.getAll("c").length === 1 && params.getAll("card").length === 1) return path;
  // AssistantPage::Group uses c=<group UUID>, without a card selector.
  if (proposal.version !== 2 && url.pathname === "/assistant" &&
    /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(params.get("c") ?? "") &&
    [...params.keys()].every((key) => key === "c") && params.getAll("c").length === 1) return path;
}
