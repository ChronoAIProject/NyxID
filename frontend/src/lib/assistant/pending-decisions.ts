import type {
  NyxAgentAccessMode,
  NyxAgentAcknowledgement,
  NyxAgentApproval,
} from "@/schemas/assistant-nyxagent";
import type { SharedConsent } from "@/schemas/assistant-sharing";
import type { z } from "zod";
import type { authoredSkillPreviewSchema } from "@/schemas/agent-skills";

export type AuthoredDecisionPreview = { readonly data?: z.infer<typeof authoredSkillPreviewSchema> };

export type PendingChatDecision =
  | { readonly id: string; readonly kind: "acknowledgement"; readonly request: NyxAgentAcknowledgement }
  | { readonly id: string; readonly kind: "approval"; readonly request: NyxAgentApproval }
  | { readonly id: string; readonly kind: "consent"; readonly request: SharedConsent };

export function ownerConfirmationBypassed(row: NyxAgentAcknowledgement, accessMode?: NyxAgentAccessMode): boolean {
  return !row.confirmation?.image_publication && accessMode === "autopilot" && row.decider === "user" &&
    !["org_connection", "delegation"].includes(row.confirmation?.reason ?? "");
}

/** Presentation only: each card and the server retain their decision checks. */
export function pendingChatDecisions({ acknowledgements, approvals, consents, authoredPreviews, accessMode, now }: {
  readonly acknowledgements: readonly NyxAgentAcknowledgement[];
  readonly approvals: readonly NyxAgentApproval[];
  readonly consents: readonly SharedConsent[];
  readonly authoredPreviews?: ReadonlyMap<string, AuthoredDecisionPreview>;
  readonly accessMode?: NyxAgentAccessMode;
  readonly now: number;
}): PendingChatDecision[] {
  const decisions: Exclude<PendingChatDecision, { readonly kind: "consent" }>[] = [];
  const newestAuthored = new Map<string, NyxAgentAcknowledgement>();
  for (const request of acknowledgements) {
    const ref = request.authored_skill;
    if (!ref) continue;
    const key = `${ref.agent_id}:${ref.proposal_id}`;
    const previous = newestAuthored.get(key);
    const revision = ref.revision - (previous?.authored_skill?.revision ?? -1);
    const created = (Date.parse(request.created_at) || 0) - (Date.parse(previous?.created_at ?? "") || 0);
    if (!previous || revision > 0 || (revision === 0 && (created > 0 || (created === 0 && request.id > previous.id)))) {
      newestAuthored.set(key, request);
    }
  }
  for (const request of acknowledgements) {
    if (request.status === "pending" && request.decider === "user" &&
      request.decided_by !== "access_mode" && Date.parse(request.expires_at) > now &&
      !ownerConfirmationBypassed(request, accessMode)) {
      const ref = request.authored_skill;
      if (ref && newestAuthored.get(`${ref.agent_id}:${ref.proposal_id}`)?.id !== request.id) continue;
      const preview = request.authored_skill && authoredPreviews?.get(request.id)?.data;
      if (!preview || (preview.state === "active" && preview.actions.some((action) => ["publish", "deny", "dismiss_card"].includes(action)))) {
        decisions.push({ id: `acknowledgement:${request.id}`, kind: "acknowledgement", request });
      }
    }
  }
  for (const request of approvals) {
    const expiry = Date.parse(request.expires_at);
    if ((!request.status || request.status === "pending") && (!Number.isFinite(expiry) || expiry > now) &&
      (!request.execution_proposal || (request.execution_proposal.state === "pending" && request.execution_proposal.supported && request.execution_proposal.can_decide))) {
      decisions.push({ id: `approval:${request.id}`, kind: "approval", request });
    }
  }
  decisions.sort((a, b) => {
    const time = Date.parse(a.request.created_at) - Date.parse(b.request.created_at);
    return time || a.id.localeCompare(b.id);
  });
  const result: PendingChatDecision[] = [...decisions];
  for (const request of consents) {
    if (request.decision === "pending" &&
      (request.expires_at === null || Date.parse(request.expires_at) > now)) {
      result.push({ id: `consent:${request.id}`, kind: "consent", request });
    }
  }
  return result;
}

export function decisionMessageId(decision: PendingChatDecision): string | undefined {
  return decision.kind === "consent" ? undefined : `nyxagent-${decision.kind}:${decision.request.id}`;
}

export function nextActiveDecision(active: string | undefined, previous: readonly string[], current: readonly string[]): string | undefined {
  if (active && current.includes(active)) return active;
  const index = previous.indexOf(active ?? "");
  return previous.slice(index + 1).find((id) => current.includes(id))
    ?? previous.slice(0, Math.max(0, index)).reverse().find((id) => current.includes(id))
    ?? current[0];
}
