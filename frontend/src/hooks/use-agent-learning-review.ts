import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import {
  learningApprovalSchema,
  learningProposalsSchema,
  learningStatusSchema,
  skillDraftValidationSchema,
  type SkillPin,
} from "@/schemas/agent-skills";

/** The complete draft the edit and validate routes take. */
export interface LearningDraftBody {
  readonly schema_version: number;
  readonly kind: string;
  readonly name: string;
  readonly description: string;
  readonly skill_md: string;
  readonly files: readonly { readonly path: string; readonly content: string }[];
  readonly base_skill: SkillPin | null;
  readonly rationale: string;
  readonly safety_notes: string;
}

export function useAgentLearningReview(id: string, enabled = true) {
  const owner = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const status = useQuery({ queryKey: ["agent-learning-status", owner, id], queryFn: async () => learningStatusSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning`)), enabled: Boolean(owner && enabled), retry: false });
  const query = useQuery({ queryKey: ["agent-learning-proposals", owner, id], queryFn: async () => learningProposalsSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals`)), enabled: Boolean(owner && enabled), retry: false });
  const invalidate = () => client.invalidateQueries({ queryKey: ["agent-learning-proposals", owner, id] });
  const run = useMutation({ mutationFn: () => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/run`, { method: "POST", body: {} }), onSuccess: invalidate });
  // The server fences a reject on the reviewed revision (and defaults the reason).
  const reject = useMutation({ mutationFn: ({ proposalId, revision }: { proposalId: string; revision: number }) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}/reject`, { method: "POST", body: { revision } }), onSuccess: invalidate });
  const approve = useMutation({ mutationFn: async ({ proposalId, acknowledgementId }: { proposalId: string; acknowledgementId?: string }) => learningApprovalSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}/approve`, { method: "POST", body: acknowledgementId ? { acknowledgement_id: acknowledgementId } : {} })), onSuccess: invalidate });
  // Learned drafts rebuild from the panel; authored drafts from their card.
  const reprepare = useMutation({ mutationFn: ({ proposalId }: { proposalId: string }) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}/reprepare`, { method: "POST", body: {} }), onSuccess: invalidate });
  const edit = useMutation({ mutationFn: ({ proposalId, revision, draft }: { proposalId: string; revision: number; draft: LearningDraftBody }) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}`, { method: "PUT", body: { revision, draft } }), onSuccess: invalidate });
  // Checks a draft against the proposal's validator without saving it.
  const validate = useMutation({ mutationFn: async ({ proposalId, draft }: { proposalId: string; draft: LearningDraftBody }) => skillDraftValidationSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}/validate`, { method: "POST", body: { draft } })) });
  const configure = useMutation({ mutationFn: (body: { enabled: boolean; threshold: number }) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning`, { method: "PUT", body }), onSuccess: () => { void status.refetch(); invalidate(); } });
  const consent = useMutation({ mutationFn: (enabled: boolean) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/consent`, { method: "PUT", body: { enabled, threshold: 1 } }), onSuccess: () => void status.refetch() });
  return { ...query, status, run, reject, approve, reprepare, edit, validate, configure, consent };
}
