import { useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { ApiError } from "@/lib/api-client";
import { orgAccessSchema, type OrgAccessDraft } from "@/schemas/org-agent-access";
import { useAuthStore } from "@/stores/auth-store";
import { useAgentOwnerMutation } from "./use-agent-owner-mutation";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";
import { orgMembersQueryKey } from "./use-org-members";

export function useOrgAgentAccess(orgId: string, agentId: string) {
  const userId = useAuthStore((state) => state.user?.id);
  const client = useQueryClient();
  const key = [...nyxBotQueryKeys.root(userId), "org-access", orgId, agentId];
  const path = `/assistant/nyxagent/agents/${encodeURIComponent(agentId)}/org-access`;
  const policy = useQuery({
    queryKey: key,
    enabled: Boolean(userId),
    retry: false,
    gcTime: 0,
    queryFn: async ({ signal }) => {
      try {
        return orgAccessSchema.parse(await assistantJson(path, { signal }));
      } catch (error) {
        if (error instanceof ApiError && (error.status === 403 || error.status === 404)) {
          void client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(userId) });
          return null;
        }
        throw error;
      }
    },
  });
  const replace = useAgentOwnerMutation({
    mutationFn: async (body: OrgAccessDraft & { expected_revision: number }, authorityGuard) => {
      try {
        const saved = orgAccessSchema.parse(await assistantJson(path, { method: "PUT", body, authorityGuard }));
        client.setQueryData(key, saved);
        return saved;
      } catch (error) {
        if (error instanceof ApiError && (error.status === 403 || error.status === 404)) {
          client.setQueryData(key, error.errorResponse.error === "org_agent_access_authoring_unavailable"
            && policy.data ? { ...policy.data, authoring_available: false } : null);
        }
        throw error;
      }
    },
    onSettled: (owner) => Promise.all([
      client.invalidateQueries({ queryKey: nyxBotQueryKeys.root(owner) }),
      client.invalidateQueries({ queryKey: orgMembersQueryKey(orgId) }),
    ]),
  });
  return { policy, replace };
}
