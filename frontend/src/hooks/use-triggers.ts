import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  createTriggerResponseSchema,
  deleteTriggerResponseSchema,
  listTriggersResponseSchema,
  rotateTriggerSecretResponseSchema,
  triggerResponseSchema,
  updateTriggerResponseSchema,
  type CreateTriggerRequest,
  type CreateTriggerResponse,
  type DeleteTriggerResponse,
  type ListTriggersResponse,
  type RotateTriggerSecretResponse,
  type TriggerResponse,
  type UpdateTriggerRequest,
  type UpdateTriggerResponse,
} from "@/schemas/triggers";

const triggerQueryKey = ["triggers"] as const;

function triggerPath(id: string): string {
  return `/triggers/${encodeURIComponent(id)}`;
}

export function useTriggers(orgId?: string) {
  return useQuery({
    queryKey: [...triggerQueryKey, { orgId: orgId ?? null }],
    queryFn: async (): Promise<ListTriggersResponse> => {
      const path = orgId
        ? `/triggers?org_id=${encodeURIComponent(orgId)}`
        : "/triggers";
      const response = await api.get<ListTriggersResponse>(path);
      return listTriggersResponseSchema.parse(response);
    },
  });
}

export function useTrigger(id: string) {
  return useQuery({
    queryKey: [...triggerQueryKey, id],
    queryFn: async (): Promise<TriggerResponse> => {
      const response = await api.get<TriggerResponse>(triggerPath(id));
      return triggerResponseSchema.parse(response);
    },
    enabled: id.length > 0,
  });
}

function useInvalidateTriggers() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: triggerQueryKey });
}

export function useCreateTrigger() {
  const invalidate = useInvalidateTriggers();
  return useMutation({
    mutationFn: async (
      data: CreateTriggerRequest,
    ): Promise<CreateTriggerResponse> => {
      const response = await api.post<CreateTriggerResponse>("/triggers", data);
      return createTriggerResponseSchema.parse(response);
    },
    onSuccess: () => {
      void invalidate();
    },
  });
}

export function useUpdateTrigger() {
  const invalidate = useInvalidateTriggers();
  return useMutation({
    mutationFn: async ({
      id,
      data,
    }: {
      readonly id: string;
      readonly data: UpdateTriggerRequest;
    }): Promise<UpdateTriggerResponse> => {
      const response = await api.patch<UpdateTriggerResponse>(
        triggerPath(id),
        data,
      );
      return updateTriggerResponseSchema.parse(response);
    },
    onSuccess: () => {
      void invalidate();
    },
  });
}

export function useDeleteTrigger() {
  const invalidate = useInvalidateTriggers();
  return useMutation({
    mutationFn: async (id: string): Promise<DeleteTriggerResponse> => {
      const response = await api.delete<DeleteTriggerResponse>(triggerPath(id));
      return deleteTriggerResponseSchema.parse(response);
    },
    onSuccess: () => {
      void invalidate();
    },
  });
}

export function useRotateTriggerSecret() {
  const invalidate = useInvalidateTriggers();
  return useMutation({
    mutationFn: async (id: string): Promise<RotateTriggerSecretResponse> => {
      const response = await api.post<RotateTriggerSecretResponse>(
        `${triggerPath(id)}/rotate-secret`,
      );
      return rotateTriggerSecretResponseSchema.parse(response);
    },
    onSuccess: () => {
      void invalidate();
    },
  });
}
