import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import {
  agentSkillsSchema,
  skillSearchSchema,
  skillVersionsSchema,
  skillPreviewSchema,
  type AgentSkill,
} from "@/schemas/agent-skills";
const base = "/assistant/nyxagent";
export function useAgentSkills(id: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["agent-skills", owner, id],
    queryFn: async () =>
      agentSkillsSchema.parse(
        await assistantJson(`${base}/agents/${encodeURIComponent(id)}/skills`),
      ),
    enabled: Boolean(owner),
    retry: false,
  });
}
export function useSetAgentSkills(id: string) {
  const client = useQueryClient();
  const owner = useAuthStore((s) => s.user?.id);
  return useMutation({
    mutationFn: (selection: {
      expected_revision: number;
      skills: AgentSkill[];
    }) =>
      assistantJson(`${base}/agents/${encodeURIComponent(id)}/skills`, {
        method: "PUT",
        body: selection,
      }),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: ["agent-skills", owner, id] }),
  });
}
export function useSkillSearch(query: string | null, page: number) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["ornn-skills", owner, query, page],
    queryFn: async () =>
      skillSearchSchema.parse(
        await assistantJson(
          `${base}/skills/catalog?q=${encodeURIComponent(query ?? "")}&page=${page}`,
        ),
      ),
    enabled: Boolean(owner) && query !== null,
    retry: false,
  });
}
export function useSkillVersions(id: string | undefined, page = 1) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["ornn-versions", owner, id, page],
    queryFn: async () =>
      skillVersionsSchema.parse(
        await assistantJson(
          `${base}/skills/catalog?skill=${encodeURIComponent(id!)}&page=${page}`,
        ),
      ),
    enabled: Boolean(owner && id),
    retry: false,
  });
}
export function useSkillPreview(id: string | undefined, version: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["ornn-preview", owner, id, version],
    queryFn: async () =>
      skillPreviewSchema.parse(
        await assistantJson(
          `${base}/skills/catalog?skill=${encodeURIComponent(id!)}&version=${encodeURIComponent(version)}`,
        ),
      ),
    enabled: Boolean(owner && id && version),
    retry: false,
  });
}
