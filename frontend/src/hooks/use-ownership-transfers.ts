import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type {
  OwnershipResourceKind,
  OwnershipResource,
  OwnershipResourceList,
  OwnershipDestination,
  OwnershipTransferPreview,
  OwnershipTransferRequest,
  OwnershipTransferResult,
} from "@/types/ownership-transfers";

export function useOwnershipTransferPreview(
  kind: OwnershipResourceKind,
  id: string,
) {
  return useMutation({
    mutationFn: (newOwnerId: string) =>
      api.post<OwnershipTransferPreview>(`/ownership/${kind}/${id}/preview`, {
        new_owner_user_id: newOwnerId,
      }),
  });
}

export function useOwnershipTransfer(kind: OwnershipResourceKind, id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: OwnershipTransferRequest) =>
      api.post<OwnershipTransferResult>(
        `/ownership/${kind}/${id}/transfer`,
        body,
      ),
    onSuccess: () => {
      for (const queryKey of [
        ["admin", "ownership"],
        ["ownership"],
        ["services"],
        ["provider-services"],
        ["catalog"],
        ["external-api-keys"],
        ["channel-bots"],
        ["channel-conversations"],
      ]) {
        void client.invalidateQueries({ queryKey });
      }
    },
  });
}

export function useOwnershipResources(
  kind: OwnershipResourceKind,
  search: string,
  offset: number,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ["admin", "ownership", kind, search, offset],
    queryFn: () =>
      api.get<OwnershipResourceList>(
        `/admin/ownership/${kind}?${new URLSearchParams({ search, offset: String(offset) })}`,
      ),
    enabled,
  });
}

export function useOwnershipTransferAuthorization(
  kind: OwnershipResourceKind,
  id: string,
  actorId?: string,
) {
  return useQuery({
    queryKey: ["ownership", kind, id, "authorization", actorId],
    queryFn: () =>
      api.get<{ can_transfer: boolean; resource: OwnershipResource | null }>(
        `/ownership/${kind}/${id}/authorization`,
      ),
    enabled: Boolean(actorId) && Boolean(id),
  });
}

export function useOwnershipDestinations(
  kind: OwnershipResourceKind,
  id: string,
  page: number,
  search: string,
  ownerType: "person" | "org",
  enabled: boolean,
) {
  const actorId = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["ownership", kind, id, "destinations", actorId, page, search, ownerType],
    queryFn: () =>
      api.get<{ users: readonly OwnershipDestination[]; total: number }>(
        `/ownership/${kind}/${id}/destinations?${new URLSearchParams({ user_type: ownerType, search, offset: String((page - 1) * 20) })}`,
      ),
    enabled: enabled && Boolean(actorId),
  });
}
