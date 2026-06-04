import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  NodeInfo,
  NodeListResponse,
  NodeAdminInfo,
  NodeAdminsResponse,
  CreateRegistrationTokenResponse,
  RotateNodeTokenResponse,
  TransferNodeResponse,
  NodePendingCredentialInfo,
  NodePendingCredentialsResponse,
  NodePendingCredentialPubkeyResponse,
  NodePendingCredentialCiphertextResponse,
} from "@/types/nodes";
import type { CiphertextEnvelope } from "@/lib/crypto";
import type {
  CreateRegistrationTokenFormData,
  TransferNodeFormData,
  PushNodeCredentialFormData,
} from "@/schemas/nodes";

// --- Query hooks ---

interface MyBoundServicesResponse {
  readonly service_ids: readonly string[];
}

export function useMyNodeBindings() {
  return useQuery({
    queryKey: ["nodes", "my-bindings"],
    queryFn: async (): Promise<readonly string[]> => {
      const res = await api.get<MyBoundServicesResponse>("/nodes/my-bindings");
      return res.service_ids;
    },
  });
}

export function useNodes() {
  return useQuery({
    queryKey: ["nodes"],
    queryFn: async (): Promise<readonly NodeInfo[]> => {
      const res = await api.get<NodeListResponse>("/nodes");
      return res.nodes;
    },
  });
}

export function useNode(nodeId: string) {
  return useQuery({
    queryKey: ["nodes", nodeId],
    queryFn: async (): Promise<NodeInfo> => {
      return api.get<NodeInfo>(`/nodes/${nodeId}`);
    },
    enabled: Boolean(nodeId),
  });
}

export function useNodeAdmins(nodeId: string) {
  return useQuery({
    queryKey: ["nodes", nodeId, "admins"],
    queryFn: async (): Promise<readonly NodeAdminInfo[]> => {
      const res = await api.get<NodeAdminsResponse>(`/nodes/${nodeId}/admins`);
      return res.admins;
    },
    enabled: Boolean(nodeId),
  });
}

export function useNodePendingCredentials(
  nodeId: string,
  enabled = true,
  includeHistory = false,
) {
  return useQuery({
    queryKey: ["nodes", nodeId, "pending-credentials", { includeHistory }],
    queryFn: async (): Promise<readonly NodePendingCredentialInfo[]> => {
      const suffix = includeHistory ? "?include_history=true" : "";
      const res = await api.get<NodePendingCredentialsResponse>(
        `/nodes/${nodeId}/credentials/pending${suffix}`,
      );
      return res.pending_credentials;
    },
    enabled: enabled && Boolean(nodeId),
  });
}

export function useNodePendingCredentialPubkey(
  nodeId: string,
  pendingCredentialId: string,
  enabled = true,
) {
  return useQuery({
    queryKey: [
      "nodes",
      nodeId,
      "pending-credentials",
      pendingCredentialId,
      "pubkey",
    ],
    queryFn: async (): Promise<NodePendingCredentialPubkeyResponse> => {
      return api.get<NodePendingCredentialPubkeyResponse>(
        `/nodes/${nodeId}/credentials/pending/${pendingCredentialId}`,
      );
    },
    enabled: enabled && Boolean(nodeId) && Boolean(pendingCredentialId),
  });
}

// --- Mutation hooks ---

export function useCreateRegistrationToken() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateRegistrationTokenFormData,
    ): Promise<CreateRegistrationTokenResponse> => {
      return api.post<CreateRegistrationTokenResponse>(
        "/nodes/register-token",
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}

export function useDeleteNode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (nodeId: string): Promise<void> => {
      return api.delete<void>(`/nodes/${nodeId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}

export function useRotateNodeToken() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (nodeId: string): Promise<RotateNodeTokenResponse> => {
      return api.post<RotateNodeTokenResponse>(`/nodes/${nodeId}/rotate-token`);
    },
    onSuccess: (_data, nodeId) => {
      void queryClient.invalidateQueries({ queryKey: ["nodes", nodeId] });
    },
  });
}

export function useTransferNode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      nodeId,
      data,
    }: {
      readonly nodeId: string;
      readonly data: TransferNodeFormData;
    }): Promise<TransferNodeResponse> => {
      return api.post<TransferNodeResponse>(
        `/nodes/${nodeId}/transfer`,
        data,
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
      void queryClient.invalidateQueries({
        queryKey: ["nodes", variables.nodeId],
      });
      void queryClient.invalidateQueries({
        queryKey: ["nodes", variables.nodeId, "admins"],
      });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}

export function usePushNodeCredential(nodeId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: PushNodeCredentialFormData,
    ): Promise<NodePendingCredentialInfo> => {
      return api.post<NodePendingCredentialInfo>(
        `/nodes/${nodeId}/credentials/push`,
        data,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["nodes", nodeId, "pending-credentials"],
      });
    },
  });
}

export function usePostNodePendingCredentialCiphertext(
  nodeId: string,
  pendingCredentialId: string,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      envelope: CiphertextEnvelope,
    ): Promise<NodePendingCredentialCiphertextResponse> => {
      return api.post<NodePendingCredentialCiphertextResponse>(
        `/nodes/${nodeId}/credentials/pending/${pendingCredentialId}/ciphertext`,
        envelope,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["nodes", nodeId, "pending-credentials"],
      });
    },
  });
}

export function useCancelNodePendingCredential(nodeId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (pendingCredentialId: string): Promise<void> => {
      return api.delete<void>(
        `/nodes/${nodeId}/credentials/pending/${pendingCredentialId}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["nodes", nodeId, "pending-credentials"],
      });
    },
  });
}
