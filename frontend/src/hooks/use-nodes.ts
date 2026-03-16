import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  NodeInfo,
  NodeListResponse,
  NodeBindingInfo,
  BindingListResponse,
  CreateRegistrationTokenResponse,
  RotateNodeTokenResponse,
  CreateBindingResponse,
} from "@/types/nodes";
import type { CreateRegistrationTokenFormData } from "@/schemas/nodes";

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

export function useNodeBindings(nodeId: string) {
  return useQuery({
    queryKey: ["nodes", nodeId, "bindings"],
    queryFn: async (): Promise<readonly NodeBindingInfo[]> => {
      const res = await api.get<BindingListResponse>(
        `/nodes/${nodeId}/bindings`,
      );
      return res.bindings;
    },
    enabled: Boolean(nodeId),
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

export function useCreateBinding() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      nodeId,
      serviceId,
    }: {
      readonly nodeId: string;
      readonly serviceId: string;
    }): Promise<CreateBindingResponse> => {
      return api.post<CreateBindingResponse>(`/nodes/${nodeId}/bindings`, {
        service_id: serviceId,
      });
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["nodes", variables.nodeId, "bindings"],
      });
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}

export function useUpdateBinding() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      nodeId,
      bindingId,
      priority,
    }: {
      readonly nodeId: string;
      readonly bindingId: string;
      readonly priority: number;
    }): Promise<void> => {
      return api.patch<void>(`/nodes/${nodeId}/bindings/${bindingId}`, {
        priority,
      });
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["nodes", variables.nodeId, "bindings"],
      });
    },
  });
}

export function useDeleteBinding() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      nodeId,
      bindingId,
    }: {
      readonly nodeId: string;
      readonly bindingId: string;
    }): Promise<void> => {
      return api.delete<void>(`/nodes/${nodeId}/bindings/${bindingId}`);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["nodes", variables.nodeId, "bindings"],
      });
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}
