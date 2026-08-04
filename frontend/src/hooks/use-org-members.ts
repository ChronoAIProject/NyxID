import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AddMemberRequest,
  MemberListResponse,
  MemberResponse,
  UpdateMemberRequest,
} from "@/schemas/orgs";
import { orgsQueryKeys } from "./use-orgs";

export const orgMembersQueryKey = (orgId: string) =>
  [...orgsQueryKeys.detail(orgId), "members"] as const;

export function useOrgMembers(orgId: string) {
  return useQuery({
    queryKey: orgMembersQueryKey(orgId),
    queryFn: async (): Promise<readonly MemberResponse[]> => {
      const res = await api.get<MemberListResponse>(`/orgs/${orgId}/members`);
      return res.members;
    },
    enabled: Boolean(orgId),
  });
}

interface AddMemberParams {
  readonly orgId: string;
  readonly body: AddMemberRequest;
}

export function useAddMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      orgId,
      body,
    }: AddMemberParams): Promise<MemberResponse> => {
      return api.post<MemberResponse>(`/orgs/${orgId}/members`, body);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: orgMembersQueryKey(variables.orgId),
      });
      void queryClient.invalidateQueries({
        queryKey: orgsQueryKeys.detail(variables.orgId),
      });
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}

interface UpdateMemberParams {
  readonly orgId: string;
  readonly memberId: string;
  readonly body: UpdateMemberRequest;
}

export function useUpdateMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      orgId,
      memberId,
      body,
    }: UpdateMemberParams): Promise<MemberResponse> => {
      return api.patch<MemberResponse>(
        `/orgs/${orgId}/members/${memberId}`,
        body,
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: orgMembersQueryKey(variables.orgId),
      });
      // Also invalidate the org detail (drives `your_role`) and the
      // org list. Self-demote is the case that motivates this: if an
      // admin demotes themselves and we don't refresh the detail
      // query, the page keeps showing admin-only tabs (Approvals /
      // Settings / Invite) until a hard refresh, and every follow-up
      // action 403s.
      void queryClient.invalidateQueries({
        queryKey: orgsQueryKeys.detail(variables.orgId),
      });
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}

interface RemoveMemberParams {
  readonly orgId: string;
  readonly memberId: string;
}

export function useRemoveMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      orgId,
      memberId,
    }: RemoveMemberParams): Promise<void> => {
      return api.delete<void>(`/orgs/${orgId}/members/${memberId}`);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({
        queryKey: orgMembersQueryKey(variables.orgId),
      });
      void queryClient.invalidateQueries({
        queryKey: orgsQueryKeys.detail(variables.orgId),
      });
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}
