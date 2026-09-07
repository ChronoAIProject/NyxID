import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, apiClient } from "@/lib/api-client";
import { approveResponseSchema, userCodeSchema } from "@/schemas/auth-device";
import {
  agentKeyApproveSchema,
  agentKeyOptionsSchema,
  agentKeyPreviewSchema,
  loginCredentialsSchema,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";

export async function previewAgentKey(userCode: string) {
  return agentKeyPreviewSchema.parse(
    await apiClient("/auth/agent-key/preview", {
      method: "POST",
      body: { user_code: userCodeSchema.parse(userCode) },
      credentials: "omit",
      preserveSessionOn401: true,
    }),
  );
}
export function usePreviewAgentKeyLogin() {
  return useMutation({ gcTime: 0, mutationFn: previewAgentKey });
}
export function useAgentKeyLoginOptions() {
  return useMutation({
    gcTime: 0,
    mutationFn: async (userCode: string) =>
      agentKeyOptionsSchema.parse(
        await api.post("/auth/agent-key/options", {
          user_code: userCodeSchema.parse(userCode),
        }),
      ),
  });
}
export function useApproveAgentKeyLogin() {
  return useMutation({
    gcTime: 0,
    mutationFn: async (input: AgentKeyApprove) =>
      approveResponseSchema.parse(
        await api.post(
          "/auth/agent-key/approve",
          agentKeyApproveSchema.parse(input),
        ),
      ),
  });
}
export function useDenyAgentKeyLogin() {
  return useMutation({
    gcTime: 0,
    mutationFn: async (userCode: string) =>
      approveResponseSchema.parse(
        await api.post("/auth/agent-key/deny", {
          user_code: userCodeSchema.parse(userCode),
        }),
      ),
  });
}
export function useLoginCredentials(keyId: string) {
  return useQuery({
    queryKey: ["api-keys", keyId, "credentials"],
    queryFn: async () =>
      loginCredentialsSchema.parse(
        await api.get(`/api-keys/${encodeURIComponent(keyId)}/credentials`),
      ).credentials,
  });
}
export function useRevokeLoginCredential(keyId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.delete(
        `/api-keys/${encodeURIComponent(keyId)}/credentials/${encodeURIComponent(id)}`,
      ),
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: ["api-keys", keyId, "credentials"],
      }),
  });
}
