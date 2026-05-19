import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { OrgRole } from "@/schemas/orgs";
import type {
  OrgRoleScope,
  OrgRoleScopesResponse,
  UpdateRoleScopeRequest,
} from "@/schemas/org-role-scopes";
import { orgMembersQueryKey } from "./use-org-members";
import { orgsQueryKeys } from "./use-orgs";

export const orgRoleScopesQueryKey = (orgId: string) =>
  [...orgsQueryKeys.detail(orgId), "role-scopes"] as const;

export function useOrgRoleScopes(orgId: string) {
  return useQuery({
    queryKey: orgRoleScopesQueryKey(orgId),
    queryFn: async (): Promise<readonly OrgRoleScope[]> => {
      const res = await api.get<OrgRoleScopesResponse>(
        `/orgs/${orgId}/role-scopes`,
      );
      return res.role_scopes;
    },
    enabled: Boolean(orgId),
  });
}

interface SetOrgRoleScopeParams {
  readonly role: OrgRole;
  readonly body: UpdateRoleScopeRequest;
}

export function useSetOrgRoleScope(orgId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      role,
      body,
    }: SetOrgRoleScopeParams): Promise<OrgRoleScope> => {
      return api.put<OrgRoleScope>(`/orgs/${orgId}/role-scopes/${role}`, body);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: orgRoleScopesQueryKey(orgId),
      });
      void queryClient.invalidateQueries({
        queryKey: orgMembersQueryKey(orgId),
      });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}

interface ClearOrgRoleScopeParams {
  readonly role: OrgRole;
}

export function useClearOrgRoleScope(orgId: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({ role }: ClearOrgRoleScopeParams): Promise<void> => {
      return api.delete<void>(`/orgs/${orgId}/role-scopes/${role}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: orgRoleScopesQueryKey(orgId),
      });
      void queryClient.invalidateQueries({
        queryKey: orgMembersQueryKey(orgId),
      });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}
