import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  OwnershipResourceKind,
  OwnershipResourceList,
  OwnershipTransferPreview,
  OwnershipTransferRequest,
  OwnershipTransferResult,
} from "@/types/ownership-transfers";

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

export function useOwnershipTransferPreview(
  kind: OwnershipResourceKind,
  id: string,
) {
  return useMutation({
    mutationFn: (newOwnerId: string) =>
      api.post<OwnershipTransferPreview>(
        `/admin/ownership/${kind}/${id}/preview`,
        { new_owner_user_id: newOwnerId },
      ),
  });
}

export function useOwnershipTransfer(kind: OwnershipResourceKind, id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: OwnershipTransferRequest) =>
      api.post<OwnershipTransferResult>(
        `/admin/ownership/${kind}/${id}/transfer`,
        body,
      ),
    onSuccess: () => {
      for (const queryKey of [
        ["admin", "ownership"],
        ["services"],
        ["catalog"],
        ["channel-bots"],
        ["channel-conversations"],
      ]) {
        void client.invalidateQueries({ queryKey });
      }
    },
  });
}
