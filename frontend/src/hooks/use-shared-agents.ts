import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { sharingSchema, sharedNotesSchema, sharedConsentsSchema, type SharingForm } from "@/schemas/assistant-sharing";
import { useAuthStore } from "@/stores/auth-store";
import { nyxBotQueryKeys } from "./use-nyxbot-agents";

const agentPath = (id: string) => `/assistant/nyxagent/agents/${encodeURIComponent(id)}`;
const key = (user: string | undefined, id: string, kind?: string) => ["shared-agents", user, id, ...(kind ? [kind] : [])];

export function useAgentSharing(id: string, enabled: boolean) {
  const user = useAuthStore((s) => s.user?.id);
  return useQuery({ queryKey: key(user, id, "sharing"), enabled: enabled && !!user,
    queryFn: async () => sharingSchema.parse(await assistantJson(`${agentPath(id)}/sharing`)), retry: false });
}

export function useSaveAgentSharing(id: string) {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({ mutationFn: (body: SharingForm & { expected_version: number }) =>
    assistantJson(`${agentPath(id)}/sharing`, { method: "PUT", body }),
    onSuccess: async () => { await Promise.all([
      client.invalidateQueries({ queryKey: key(user, id) }),
      client.invalidateQueries({ queryKey: nyxBotQueryKeys.agents(user) }),
    ]); } });
}

export function useSharedNotes(id: string, kind: "private-notes" | "shared-memory") {
  const user = useAuthStore((s) => s.user?.id);
  return useQuery({ queryKey: key(user, id, kind),
    queryFn: async () => sharedNotesSchema.parse(await assistantJson(`${agentPath(id)}/${kind}`)), retry: false });
}

export function useSaveSharedNotes(id: string, kind: "private-notes" | "shared-memory") {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({ mutationFn: (body: { expected_revision: number; notes: string[] }) =>
    assistantJson(`${agentPath(id)}/${kind}`, { method: "PUT", body }),
    onSuccess: () => client.invalidateQueries({ queryKey: key(user, id) }) });
}

export function useSharedConsents(id: string) {
  const user = useAuthStore((s) => s.user?.id);
  return useQuery({ queryKey: key(user, id, "consents"),
    queryFn: async () => sharedConsentsSchema.parse(await assistantJson(`${agentPath(id)}/consents`)),
    refetchInterval: 3000, retry: false });
}

export function useDecideSharedConsent(id: string) {
  const user = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({ mutationFn: ({ consent, decision }: { consent: string; decision: "once" | "always" | "skip" | "revoked" }) =>
    assistantJson(`/assistant/nyxagent/shared-consents/${encodeURIComponent(consent)}`, { method: "POST", body: { decision } }),
    onSuccess: () => client.invalidateQueries({ queryKey: key(user, id, "consents") }) });
}
