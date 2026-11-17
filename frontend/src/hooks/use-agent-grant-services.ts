import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";

const response = z.object({ services: z.array(z.object({
  id: z.string(), slug: z.string(), label: z.string(), executable: z.boolean(),
  catalog_slug: z.string().optional(), icon_url: z.string().optional(),
})) });

/** The same live authorization inventory used by create, grants and NyxBot. */
export function useAgentGrantServices(org?: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const query = useQuery({
    queryKey: ["assistant", "nyxagent", actor, "grant-services", org ?? null],
    enabled: Boolean(actor),
    retry: false,
    queryFn: async ({ signal }) => response.parse(await assistantJson(
      `/assistant/nyxagent/grant-services${org ? `?org=${encodeURIComponent(org)}` : ""}`,
      { signal, authorityGuard: () => {
        if (!actor || useAuthStore.getState().user?.id !== actor)
          throw new Error("Account changed before loading grants");
      } },
    )).services,
    staleTime: 0,
    refetchOnMount: "always",
  });
  return { ...query, data: query.isError ? undefined : query.data };
}
