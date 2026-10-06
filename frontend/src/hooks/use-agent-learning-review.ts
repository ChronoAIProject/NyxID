import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import {
  learningApprovalSchema,
  learningProposalsSchema,
  learningStatusSchema,
} from "@/schemas/agent-skills";

export function useAgentLearningReview(id: string, enabled = true) {
  const owner = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const status = useQuery({ queryKey: ["agent-learning-status", owner, id], queryFn: async () => learningStatusSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning`)), enabled: Boolean(owner && enabled), retry: false });
  const query = useQuery({ queryKey: ["agent-learning-proposals", owner, id], queryFn: async () => learningProposalsSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals`)), enabled: Boolean(owner && enabled), retry: false });
  const invalidate = () => client.invalidateQueries({ queryKey: ["agent-learning-proposals", owner, id] });
  const run = useMutation({ mutationFn: () => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/run`, { method: "POST", body: {} }), onSuccess: invalidate });
  const reject = useMutation({ mutationFn: (proposalId: string) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}/reject`, { method: "POST", body: {} }), onSuccess: invalidate });
  const approve = useMutation({ mutationFn: async ({ proposalId, acknowledgementId }: { proposalId: string; acknowledgementId?: string }) => learningApprovalSchema.parse(await assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}/approve`, { method: "POST", body: acknowledgementId ? { acknowledgement_id: acknowledgementId } : {} })), onSuccess: invalidate });
  const edit = useMutation({ mutationFn: ({ proposalId, draft }: { proposalId: string; draft: unknown }) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning/proposals/${encodeURIComponent(proposalId)}`, { method: "PUT", body: { draft } }), onSuccess: invalidate });
  const configure = useMutation({ mutationFn: (body: { enabled: boolean; threshold: number }) => assistantJson(`/assistant/nyxagent/agents/${encodeURIComponent(id)}/learning`, { method: "PUT", body }), onSuccess: () => { void status.refetch(); invalidate(); } });
  return { ...query, status, run, reject, approve, edit, configure };
}
