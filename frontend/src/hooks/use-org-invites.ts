import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  CreateInviteRequest,
  InviteListResponse,
  InviteResponse,
} from "@/schemas/orgs";
import { orgsQueryKeys } from "./use-orgs";

export const orgInvitesQueryKey = (orgId: string) =>
  [...orgsQueryKeys.detail(orgId), "invites"] as const;

export function useOrgInvites(orgId: string) {
  return useQuery({
    queryKey: orgInvitesQueryKey(orgId),
    queryFn: async (): Promise<readonly InviteResponse[]> => {
      const res = await api.get<InviteListResponse>(`/orgs/${orgId}/invites`);
      return res.invites;
    },
    enabled: Boolean(orgId),
  });
}

interface CreateInviteParams {
  readonly orgId: string;
  readonly body: CreateInviteRequest;
}

export function useCreateInvite() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      orgId,
      body,
    }: CreateInviteParams): Promise<InviteResponse> => {
      return api.post<InviteResponse>(`/orgs/${orgId}/invites`, body);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: orgInvitesQueryKey(variables.orgId),
      });
    },
  });
}

interface CancelInviteParams {
  readonly orgId: string;
  readonly inviteId: string;
}

export function useCancelInvite() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      orgId,
      inviteId,
    }: CancelInviteParams): Promise<void> => {
      return api.delete<void>(`/orgs/${orgId}/invites/${inviteId}`);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: orgInvitesQueryKey(variables.orgId),
      });
    },
  });
}
