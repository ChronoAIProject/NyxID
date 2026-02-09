import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { DownstreamService, UserServiceConnection } from "@/types/api";
import type { CreateServiceFormData } from "@/schemas/services";

export function useServices() {
  return useQuery({
    queryKey: ["services"],
    queryFn: async (): Promise<readonly DownstreamService[]> => {
      return api.get<readonly DownstreamService[]>("/services");
    },
  });
}

export function useCreateService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateServiceFormData,
    ): Promise<DownstreamService> => {
      return api.post<DownstreamService>("/services", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["services"] });
    },
  });
}

export function useDeleteService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/services/${id}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["services"] });
    },
  });
}

export function useConnections() {
  return useQuery({
    queryKey: ["connections"],
    queryFn: async (): Promise<readonly UserServiceConnection[]> => {
      return api.get<readonly UserServiceConnection[]>("/connections");
    },
  });
}

export function useConnectService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (serviceId: string): Promise<UserServiceConnection> => {
      return api.post<UserServiceConnection>(`/connections/${serviceId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}

export function useDisconnectService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (serviceId: string): Promise<void> => {
      return api.delete<void>(`/connections/${serviceId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["connections"] });
    },
  });
}
