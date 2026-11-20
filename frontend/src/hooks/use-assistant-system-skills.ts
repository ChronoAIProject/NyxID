import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  systemSkillsSchema,
  type SystemSkillsSelection,
} from "@/schemas/assistant-system-skills";
import {
  skillSearchSchema,
  skillVersionsSchema,
  skillPreviewSchema,
} from "@/schemas/agent-skills";
const path = "/admin/assistant/system-skills";
export function useSystemSkills() {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["admin-system-skills", owner],
    queryFn: async () => systemSkillsSchema.parse(await api.get(path)),
    retry: false,
    refetchOnWindowFocus: false,
  });
}
export function useSetSystemSkills() {
  const owner = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (selection: SystemSkillsSelection) =>
      systemSkillsSchema.parse(await api.put(path, selection)),
    onSuccess: (data) => {
      client.setQueryData(["admin-system-skills", owner], data);
      void client.invalidateQueries({ queryKey: ["agent-skills", owner] });
    },
  });
}

export function useSystemSkillSearch(query: string | null, page: number) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["admin-system-skill-search", owner, query, page],
    queryFn: async () =>
      skillSearchSchema.parse(
        await api.get(
          `${path}/search?q=${encodeURIComponent(query ?? "")}&page=${page}`,
        ),
      ),
    enabled: Boolean(owner) && query !== null,
    retry: false,
  });
}
export function useSystemSkillVersions(id: string | undefined, page = 1) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["admin-system-skill-versions", owner, id, page],
    queryFn: async () =>
      skillVersionsSchema.parse(
        await api.get(
          `${path}/${encodeURIComponent(id!)}/versions?page=${page}`,
        ),
      ),
    enabled: Boolean(owner && id),
    retry: false,
  });
}
export function useSystemSkillPreview(id: string | undefined, version: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["admin-system-skill-preview", owner, id, version],
    queryFn: async () =>
      skillPreviewSchema.parse(
        await api.get(
          `${path}/${encodeURIComponent(id!)}/versions/${encodeURIComponent(version)}/preview`,
        ),
      ),
    enabled: Boolean(owner && id && version),
    retry: false,
  });
}
