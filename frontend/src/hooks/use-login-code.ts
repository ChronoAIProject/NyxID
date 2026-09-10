import { useMutation, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { loginCodeSchema, loginCodeStatusSchema } from "@/schemas/login-code";
import type { AgentKeyApprove } from "@/schemas/agent-key-login";

export function useMintLoginCode() {
  return useMutation({ gcTime: 0, mutationFn: async (grant:
    { auth_kind: "account_session" } | { auth_kind: "agent_key"; selection: AgentKeyApprove["selection"]; credential_expires_at?: string }) =>
    loginCodeSchema.parse(await api.post("/auth/login-code", grant)),
  });
}
export function useLoginCodeStatus(id: string) {
  return useQuery({ queryKey: ["login-code", id], gcTime: 0,
    queryFn: async () => loginCodeStatusSchema.parse(await api.get(`/auth/login-code/${encodeURIComponent(id)}`)),
    refetchInterval: (query) => query.state.data?.status === "pending" || !query.state.data ? 5000 : false,
  });
}
export function useLoginCodeAction(id: string) {
  return useMutation({ gcTime: 0, mutationFn: (action: "cancel" | "revoke") => action === "cancel"
    ? api.delete(`/auth/login-code/${encodeURIComponent(id)}`)
    : api.post(`/auth/login-code/${encodeURIComponent(id)}/revoke`),
  });
}
