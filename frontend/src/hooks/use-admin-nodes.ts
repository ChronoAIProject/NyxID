import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AdminNodeInfo,
  AdminNodeListResponse,
} from "@/types/nodes";

export function useAdminNodes(
  page: number,
  perPage: number,
  status?: string,
  search?: string,
) {
  return useQuery({
    queryKey: ["admin", "nodes", page, perPage, status, search],
    queryFn: async (): Promise<AdminNodeListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      if (status) params.set("status", status);
      if (search) params.set("user_id", search);
      return api.get<AdminNodeListResponse>(
        `/admin/nodes?${params.toString()}`,
      );
    },
  });
}

export function useAdminNode(nodeId: string) {
  return useQuery({
    queryKey: ["admin", "nodes", nodeId],
    queryFn: async (): Promise<AdminNodeInfo> => {
      return api.get<AdminNodeInfo>(`/admin/nodes/${nodeId}`);
    },
    enabled: nodeId.length > 0,
  });
}

export function useAdminDisconnectNode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (nodeId: string): Promise<void> => {
      return api.post<void>(`/admin/nodes/${nodeId}/disconnect`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "nodes"] });
    },
  });
}

export function useAdminDeleteNode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (nodeId: string): Promise<void> => {
      return api.delete<void>(`/admin/nodes/${nodeId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "nodes"] });
    },
  });
}
