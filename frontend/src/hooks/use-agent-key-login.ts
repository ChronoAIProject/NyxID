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

export type LoginFlow = "agent-key" | "device";

export async function previewAgentKey(userCode: string, flow: LoginFlow = "agent-key") {
  return agentKeyPreviewSchema.parse(
    { requested_profile: null, interval: 5, ...await apiClient<Record<string, unknown>>(`/auth/${flow}/preview`, {
      method: "POST",
      body: { user_code: userCodeSchema.parse(userCode) },
      credentials: "omit",
      preserveSessionOn401: true,
    }) },
  );
}
export function usePreviewAgentKeyLogin(flow: LoginFlow = "agent-key") {
  return useMutation({ gcTime: 0, mutationFn: (code: string) => previewAgentKey(code, flow) });
}
export function useAgentKeyLoginOptions(flow: LoginFlow = "agent-key", mint = false) {
  return useMutation({
    gcTime: 0,
    mutationFn: async (userCode: string) =>
      agentKeyOptionsSchema.parse(
        await api.post(`/auth/${mint ? "login-code" : flow}/options`, mint ? {} : {
          user_code: userCodeSchema.parse(userCode),
        }),
      ),
  });
}
export function useApproveAgentKeyLogin(flow: LoginFlow = "agent-key") {
  return useMutation({
    gcTime: 0,
    mutationFn: async (input: AgentKeyApprove) =>
      approveResponseSchema.parse(
        await api.post(
          `/auth/${flow}/${flow === "device" ? "approve-agent-key" : "approve"}`,
          agentKeyApproveSchema.parse(input),
        ),
      ),
  });
}
export function useDenyAgentKeyLogin(flow: LoginFlow = "agent-key") {
  return useMutation({
    gcTime: 0,
    mutationFn: async (userCode: string) =>
      approveResponseSchema.parse(
        await api.post(`/auth/${flow}/deny`, {
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
