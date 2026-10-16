import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import {
  personalLinkApi,
  loadPersonalBindings,
  revokePersonalBinding,
} from "@/lib/platform-personal";
import { readPersonalLink } from "@/lib/platform-personal-handoff";
import type { PersonalBinding } from "@/schemas/platform-personal";

export function usePersonalLinkPreview() {
  const handoff = readPersonalLink();
  return useQuery({
    queryKey: ["personal-link-preview", handoff?.id],
    enabled: !!handoff,
    retry: false,
    gcTime: 0,
    staleTime: 0,
    refetchOnWindowFocus: false,
    queryFn: () => {
      const current = readPersonalLink();
      if (!current || current.id !== handoff?.id)
        throw new Error("Reopen the original linking request.");
      return personalLinkApi.preview(current.token);
    },
  });
}
export function usePersonalBindings() {
  const actor = useAuthStore((s) => s.user?.id);
  return useInfiniteQuery({
    queryKey: ["platform-personal-bindings", actor],
    enabled: !!actor,
    retry: false,
    gcTime: 0,
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) => loadPersonalBindings(actor!, pageParam),
    getNextPageParam: (page) => page.next_before_id ?? undefined,
  });
}
export function useRevokePersonalBinding() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    gcTime: 0,
    retry: false,
    mutationFn: (binding: PersonalBinding) =>
      revokePersonalBinding(actor!, binding.id, binding.generation),
    onSuccess: () =>
      client.invalidateQueries({
        queryKey: ["platform-personal-bindings", actor],
      }),
  });
}
